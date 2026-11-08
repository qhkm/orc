use serde::{Deserialize, Serialize};
use crate::invoke::{execute, InvokeOpts, Provider};
use super::{tail, CheckResult};

/// The reviewer's judgement of one `--accept` criterion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CriterionResult {
    pub criterion: String,
    pub pass: bool,
    pub reason: String,
}

/// Outcome of the review step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Review {
    pub reviewer: String,
    pub results: Vec<CriterionResult>,
    /// Set when the review could not be completed (reviewer failed, or no usable answer).
    pub error: Option<String>,
}

/// Build the prompt for the reviewer agent.
pub fn build_review_prompt(task: &str, criteria: &[String], diff: &str, checks: &[CheckResult]) -> String {
    let mut prompt = String::from(
        "You are reviewing another coding agent's work. Do not modify any files.\n\
         Judge each acceptance criterion strictly against the changes below. You may read files in the repository.\n\
         A criterion passes only if the changes clearly satisfy it.\n\n",
    );
    prompt.push_str(&format!("## Task given to the agent\n\n{task}\n\n## Acceptance criteria\n\n"));
    for (i, c) in criteria.iter().enumerate() {
        prompt.push_str(&format!("{}. {}\n", i + 1, c));
    }
    if !checks.is_empty() {
        prompt.push_str("\n## Automated checks (already run by orc)\n\n");
        for c in checks {
            prompt.push_str(&format!("- `{}`: {}\n", c.command, if c.passed { "passed" } else { "FAILED" }));
        }
    }
    prompt.push_str(&format!("\n## Changes\n\n```diff\n{diff}\n```\n\n"));
    prompt.push_str(
        "Reply with JSON only, no other text, one entry per criterion id:\n\
         {\"results\": [{\"id\": 1, \"pass\": true, \"reason\": \"<one sentence>\"}]}\n",
    );
    prompt
}

#[derive(Deserialize)]
struct RawReply {
    results: Vec<RawResult>,
}

#[derive(Deserialize)]
struct RawResult {
    id: usize,
    pass: bool,
    #[serde(default)]
    reason: String,
}

/// Find the reviewer's JSON object (the first `{...}` with a `results` array) anywhere in `text`.
fn find_reply(text: &str) -> Option<RawReply> {
    for (i, _) in text.match_indices('{') {
        let mut stream = serde_json::Deserializer::from_str(&text[i..]).into_iter::<serde_json::Value>();
        if let Some(Ok(value)) = stream.next() {
            if value.get("results").is_some_and(|r| r.is_array()) {
                if let Ok(reply) = serde_json::from_value::<RawReply>(value) {
                    return Some(reply);
                }
            }
        }
    }
    None
}

/// Parse the reviewer's reply. Every criterion gets a result; one the reviewer
/// did not answer counts as a fail.
pub fn parse_review_response(text: &str, criteria: &[String]) -> Result<Vec<CriterionResult>, String> {
    let reply = find_reply(text).ok_or_else(|| {
        format!("reviewer reply had no JSON results object. Reply:\n{}", tail(text, 20, 1500))
    })?;
    Ok(criteria
        .iter()
        .enumerate()
        .map(|(i, criterion)| match reply.results.iter().find(|r| r.id == i + 1) {
            Some(r) => CriterionResult { criterion: criterion.clone(), pass: r.pass, reason: r.reason.clone() },
            None => CriterionResult {
                criterion: criterion.clone(),
                pass: false,
                reason: "reviewer gave no answer for this criterion".to_string(),
            },
        })
        .collect())
}

/// What the reviewer is asked to judge.
pub struct ReviewRequest<'a> {
    pub task: &'a str,
    pub criteria: &'a [String],
    pub diff: &'a str,
    pub checks: &'a [CheckResult],
    pub working_dir: &'a str,
    pub model: Option<String>,
    pub timeout_secs: u64,
}

/// Ask `reviewer` to judge the criteria against the diff. The reviewer runs read-only.
pub async fn review(reviewer: &dyn Provider, req: ReviewRequest<'_>) -> Review {
    let criteria = req.criteria;
    let opts = InvokeOpts {
        message: build_review_prompt(req.task, criteria, req.diff, req.checks),
        working_dir: req.working_dir.to_string(),
        model: req.model,
        timeout_secs: req.timeout_secs,
        idle_timeout_secs: Some(req.timeout_secs.min(300)),
        read_only: true,
        ..Default::default()
    };
    let name = reviewer.name().to_string();
    let failed = |error: String| Review { reviewer: name.clone(), results: vec![], error: Some(error) };

    let result = match execute(reviewer, opts).await {
        Ok(result) => result,
        Err(e) => return failed(format!("could not start reviewer: {e}")),
    };
    // Claude puts the clean final answer in the result event, which lives in raw_output.
    let parsed = parse_review_response(&result.response, criteria)
        .or_else(|_| parse_review_response(&crate::invoke::providers::claude::extract_result_meta(&result.raw_output).result_text.unwrap_or_default(), criteria))
        .or_else(|_| parse_review_response(&result.raw_output, criteria));
    match parsed {
        Ok(results) => Review { reviewer: name, results, error: None },
        Err(e) => {
            let mut error = e;
            if result.exit_code != 0 || !result.errors.is_empty() {
                error = format!(
                    "reviewer exited with code {} ({:?}): {}\n{}",
                    result.exit_code,
                    result.exit_reason,
                    result.errors.join("; "),
                    tail(&result.stderr, 10, 1000)
                );
            }
            failed(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::ShellProvider;

    fn criteria() -> Vec<String> {
        vec!["docs updated".into(), "has tests".into()]
    }

    fn req<'a>(criteria: &'a [String], dir: &'a str) -> ReviewRequest<'a> {
        ReviewRequest { task: "task", criteria, diff: "diff", checks: &[], working_dir: dir, model: None, timeout_secs: 30 }
    }

    #[test]
    fn prompt_numbers_criteria_and_includes_diff_and_checks() {
        let checks = vec![CheckResult {
            command: "cargo test".into(),
            passed: true,
            exit_code: Some(0),
            timed_out: false,
            output_tail: String::new(),
            duration_ms: 1,
        }];
        let p = build_review_prompt("add auth", &criteria(), "+fn auth() {}", &checks);
        assert!(p.contains("1. docs updated\n2. has tests"));
        assert!(p.contains("+fn auth() {}"));
        assert!(p.contains("`cargo test`: passed"));
        assert!(p.contains("Do not modify any files"));
    }

    #[test]
    fn parses_json_with_surrounding_text_and_fences() {
        let reply = "Here you go:\n```json\n{\"results\": [{\"id\": 1, \"pass\": true, \"reason\": \"README changed\"}, {\"id\": 2, \"pass\": false, \"reason\": \"no test added\"}]}\n```";
        let r = parse_review_response(reply, &criteria()).unwrap();
        assert_eq!(r[0], CriterionResult { criterion: "docs updated".into(), pass: true, reason: "README changed".into() });
        assert!(!r[1].pass);
        assert_eq!(r[1].reason, "no test added");
    }

    #[test]
    fn skips_unrelated_json_objects() {
        let reply = r#"{"note": "thinking"} then {"results": [{"id": 2, "pass": true}]}"#;
        let r = parse_review_response(reply, &criteria()).unwrap();
        assert!(!r[0].pass, "unanswered criterion fails");
        assert_eq!(r[0].reason, "reviewer gave no answer for this criterion");
        assert!(r[1].pass);
    }

    #[test]
    fn no_json_is_an_error() {
        assert!(parse_review_response("Looks good to me!", &criteria()).is_err());
    }

    #[tokio::test]
    async fn review_runs_provider_and_parses_reply() {
        let dir = tempfile::tempdir().unwrap();
        let p = ShellProvider::new(r#"echo '{"results": [{"id": 1, "pass": true, "reason": "ok"}, {"id": 2, "pass": true, "reason": "ok"}]}'"#);
        let c = criteria();
        let r = review(&p, req(&c, dir.path().to_str().unwrap())).await;
        assert_eq!(r.error, None);
        assert!(r.results.iter().all(|c| c.pass));
        assert_eq!(r.reviewer, "shell");
    }

    #[tokio::test]
    async fn failed_reviewer_is_reported_not_passed() {
        let dir = tempfile::tempdir().unwrap();
        let p = ShellProvider::new("echo 'ERROR: overloaded'; exit 1");
        let c = criteria();
        let r = review(&p, req(&c, dir.path().to_str().unwrap())).await;
        assert!(r.results.is_empty());
        assert!(r.error.unwrap().contains("overloaded"));
    }
}
