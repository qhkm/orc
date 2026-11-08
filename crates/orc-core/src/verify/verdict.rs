use serde::{Deserialize, Serialize};
use crate::invoke::{ExitReason, InvokeResult};
use super::{tail, CheckResult, Review};

/// The outcome of a stage, decided by orc.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Verdict {
    Pass,
    /// The work is not done: the agent failed, a check failed, or the reviewer rejected it.
    Fail { reason: String, evidence: String },
    /// The agent went silent or ran too long and was killed.
    Stuck { reason: String, after_secs: u64 },
    /// The provider hit a rate limit or quota.
    Limit { provider: String, message: String },
}

impl Verdict {
    /// Process exit code for `orc run`: 0 pass, 1 fail, 2 stuck, 3 limit.
    pub fn exit_code(&self) -> i32 {
        match self {
            Verdict::Pass => 0,
            Verdict::Fail { .. } => 1,
            Verdict::Stuck { .. } => 2,
            Verdict::Limit { .. } => 3,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Verdict::Pass => "pass",
            Verdict::Fail { .. } => "fail",
            Verdict::Stuck { .. } => "stuck",
            Verdict::Limit { .. } => "limit",
        }
    }
}

/// Decide a stage's verdict. First match wins:
/// stuck → limit → agent failed → check failed → review rejected/incomplete → pass.
pub fn decide(
    provider: &str,
    agent: &InvokeResult,
    limit: Option<String>,
    checks: &[CheckResult],
    review: Option<&Review>,
) -> Verdict {
    let after_secs = agent.duration_ms / 1000;
    match agent.exit_reason {
        ExitReason::Idle => {
            return Verdict::Stuck { reason: format!("{provider} stopped producing output"), after_secs };
        }
        ExitReason::Timeout => {
            return Verdict::Stuck { reason: format!("{provider} ran past the timeout"), after_secs };
        }
        ExitReason::Exited => {}
    }

    if let Some(message) = limit {
        return Verdict::Limit { provider: provider.to_string(), message };
    }

    if agent.exit_code != 0 || !agent.errors.is_empty() {
        let reason = if agent.exit_code != 0 {
            format!("{provider} exited with code {}", agent.exit_code)
        } else {
            format!("{provider} reported an error")
        };
        let mut evidence = agent.errors.join("\n");
        let stderr = tail(&agent.stderr, 20, 2000);
        if !stderr.is_empty() {
            if !evidence.is_empty() {
                evidence.push('\n');
            }
            evidence.push_str(&stderr);
        }
        return Verdict::Fail { reason, evidence };
    }

    let failed: Vec<&CheckResult> = checks.iter().filter(|c| !c.passed).collect();
    if !failed.is_empty() {
        let names: Vec<&str> = failed.iter().map(|c| c.command.as_str()).collect();
        let evidence = failed
            .iter()
            .map(|c| format!("$ {}\n{}", c.command, c.output_tail))
            .collect::<Vec<_>>()
            .join("\n\n");
        return Verdict::Fail { reason: format!("check failed: {}", names.join(", ")), evidence };
    }

    if let Some(review) = review {
        if let Some(error) = &review.error {
            return Verdict::Fail {
                reason: format!("review by {} could not be completed", review.reviewer),
                evidence: error.clone(),
            };
        }
        let rejected: Vec<&_> = review.results.iter().filter(|r| !r.pass).collect();
        if !rejected.is_empty() {
            let evidence = rejected
                .iter()
                .map(|r| format!("✗ {}: {}", r.criterion, r.reason))
                .collect::<Vec<_>>()
                .join("\n");
            return Verdict::Fail {
                reason: format!("{} rejected {} of {} criteria", review.reviewer, rejected.len(), review.results.len()),
                evidence,
            };
        }
    }

    Verdict::Pass
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verify::CriterionResult;

    fn agent(exit_code: i32, exit_reason: ExitReason) -> InvokeResult {
        InvokeResult {
            response: "done".into(),
            raw_output: String::new(),
            stderr: String::new(),
            errors: vec![],
            exit_code,
            exit_reason,
            duration_ms: 42_000,
            cost_usd: None,
        }
    }

    fn check(command: &str, passed: bool) -> CheckResult {
        CheckResult {
            command: command.into(),
            passed,
            exit_code: Some(if passed { 0 } else { 101 }),
            timed_out: false,
            output_tail: if passed { "ok".into() } else { "test add ... FAILED".into() },
            duration_ms: 10,
        }
    }

    fn review(results: &[(&str, bool)]) -> Review {
        Review {
            reviewer: "claude".into(),
            results: results
                .iter()
                .map(|(c, pass)| CriterionResult { criterion: c.to_string(), pass: *pass, reason: "because".into() })
                .collect(),
            error: None,
        }
    }

    #[test]
    fn all_good_is_pass() {
        let r = review(&[("docs updated", true)]);
        let v = decide("codex", &agent(0, ExitReason::Exited), None, &[check("cargo test", true)], Some(&r));
        assert_eq!(v, Verdict::Pass);
        assert_eq!(v.exit_code(), 0);
    }

    #[test]
    fn failing_check_is_fail_with_its_output() {
        let checks = [check("cargo fmt --check", true), check("cargo test", false)];
        let v = decide("codex", &agent(0, ExitReason::Exited), None, &checks, None);
        match &v {
            Verdict::Fail { reason, evidence } => {
                assert_eq!(reason, "check failed: cargo test");
                assert!(evidence.contains("$ cargo test"));
                assert!(evidence.contains("FAILED"));
                assert!(!evidence.contains("cargo fmt"));
            }
            other => panic!("expected fail, got {other:?}"),
        }
        assert_eq!(v.exit_code(), 1);
    }

    #[test]
    fn idle_and_timeout_are_stuck_and_win_over_everything() {
        let v = decide("claude", &agent(-1, ExitReason::Idle), Some("rate limit".into()), &[], None);
        assert!(matches!(&v, Verdict::Stuck { reason, after_secs: 42 } if reason.contains("stopped producing output")));
        assert_eq!(v.exit_code(), 2);
        let v = decide("claude", &agent(-1, ExitReason::Timeout), None, &[], None);
        assert!(matches!(&v, Verdict::Stuck { reason, .. } if reason.contains("timeout")));
    }

    #[test]
    fn limit_wins_over_agent_failure() {
        let v = decide("codex", &agent(1, ExitReason::Exited), Some("You've hit your usage limit".into()), &[], None);
        assert_eq!(v, Verdict::Limit { provider: "codex".into(), message: "You've hit your usage limit".into() });
        assert_eq!(v.exit_code(), 3);
    }

    #[test]
    fn agent_nonzero_exit_or_error_event_is_fail() {
        let mut a = agent(2, ExitReason::Exited);
        a.stderr = "panic: boom".into();
        let v = decide("codex", &a, None, &[check("cargo test", true)], None);
        assert!(matches!(&v, Verdict::Fail { reason, evidence } if reason == "codex exited with code 2" && evidence.contains("boom")));

        let mut a = agent(0, ExitReason::Exited);
        a.errors = vec!["error_max_turns".into()];
        let v = decide("claude", &a, None, &[], None);
        assert!(matches!(&v, Verdict::Fail { reason, evidence } if reason == "claude reported an error" && evidence == "error_max_turns"));
    }

    #[test]
    fn rejected_or_incomplete_review_is_fail() {
        let r = review(&[("docs updated", true), ("has tests", false)]);
        let v = decide("codex", &agent(0, ExitReason::Exited), None, &[], Some(&r));
        assert!(matches!(&v, Verdict::Fail { reason, evidence } if reason == "claude rejected 1 of 2 criteria" && evidence.contains("✗ has tests")));

        let broken = Review { reviewer: "claude".into(), results: vec![], error: Some("no JSON in reply".into()) };
        let v = decide("codex", &agent(0, ExitReason::Exited), None, &[], Some(&broken));
        assert!(matches!(&v, Verdict::Fail { reason, .. } if reason.contains("could not be completed")));
    }

    #[test]
    fn verdict_serializes_with_status_tag() {
        let json = serde_json::to_string(&Verdict::Limit { provider: "codex".into(), message: "m".into() }).unwrap();
        assert_eq!(json, r#"{"status":"limit","provider":"codex","message":"m"}"#);
        assert_eq!(serde_json::to_string(&Verdict::Pass).unwrap(), r#"{"status":"pass"}"#);
    }
}
