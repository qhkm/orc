use anyhow::Result;
use chrono::Utc;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};
use crate::capsule::{append_event, capsule_path, CapsuleEvent};
use crate::invoke::{execute_stream, ExitReason, InvokeOpts, Provider};
use crate::session::types::StreamEvent;
use super::{decide, detect_limit, review, run_check, CheckResult, Review, Verdict};

const MAX_DIFF_CHARS: usize = 60_000;
const MAX_UNTRACKED_FILE_CHARS: usize = 8_000;

/// One verified stage: an agent does `task`, then orc checks the result.
#[derive(Debug, Clone)]
pub struct StageSpec {
    pub run_id: String,
    /// The original task. The reviewer always judges against this.
    pub task: String,
    /// What the agent is sent, if different from `task` (e.g. task + handoff brief).
    pub agent_prompt: Option<String>,
    /// 1-based attempt number within the run.
    pub attempt: u32,
    /// Provider whose attempt this one follows, if any.
    pub handoff_from: Option<String>,
    /// Diff base for the review. None = HEAD when the stage starts.
    /// A relay pins the first attempt's base so later reviews see the whole run.
    pub base_commit: Option<String>,
    pub working_dir: PathBuf,
    pub model: Option<String>,
    /// `--check` shell commands.
    pub checks: Vec<String>,
    /// `--accept` plain-language criteria for the reviewer.
    pub accept: Vec<String>,
    pub reviewer_model: Option<String>,
    pub timeout_secs: u64,
    pub idle_secs: u64,
    pub check_timeout_secs: u64,
}

/// Progress callbacks while a stage runs, for live output.
#[derive(Debug, Clone)]
pub enum Progress {
    Agent(StreamEvent),
    AgentFinished { exit_code: i32, exit_reason: ExitReason, duration_ms: u64 },
    CheckStarted(String),
    CheckFinished(CheckResult),
    ReviewStarted(String),
    /// Checks and review are skipped because the agent run already decided the verdict.
    Skipped(String),
}

#[derive(Debug, Clone, Serialize)]
pub struct StageReport {
    pub run_id: String,
    pub attempt: u32,
    pub provider: String,
    pub verdict: Verdict,
    pub exit_code: i32,
    pub agent_response: String,
    pub agent_exit_code: i32,
    pub agent_exit_reason: ExitReason,
    pub checks: Vec<CheckResult>,
    pub review: Option<Review>,
    pub base_commit: Option<String>,
    pub duration_ms: u64,
    pub log_path: PathBuf,
}

/// Run one stage and decide its verdict. Every step is appended to the run log
/// at `<working_dir>/.orc/runs/<run_id>/events.jsonl`.
///
/// `reviewer` is required only when `spec.accept` is non-empty.
pub async fn run_stage(
    spec: &StageSpec,
    agent: &dyn Provider,
    reviewer: Option<&dyn Provider>,
    mut on_progress: impl FnMut(Progress) + Send,
) -> Result<StageReport> {
    anyhow::ensure!(
        spec.accept.is_empty() || reviewer.is_some(),
        "--accept criteria need a reviewer provider"
    );
    let started = Instant::now();
    let dir = spec.working_dir.as_path();
    let log_path = capsule_path(dir, &spec.run_id);
    let provider = agent.name().to_string();
    let base_commit = spec.base_commit.clone().or_else(|| git_head(dir));

    // Fail before running the agent if the record can't be written.
    append_event(
        &log_path,
        &CapsuleEvent::StageStarted {
            provider: provider.clone(),
            model: spec.model.clone(),
            task: spec.task.clone(),
            base_commit: base_commit.clone(),
            checks: spec.checks.clone(),
            accept: spec.accept.clone(),
            attempt: spec.attempt,
            handoff_from: spec.handoff_from.clone(),
            ts: Utc::now(),
        },
    )?;

    let opts = InvokeOpts {
        message: spec.agent_prompt.clone().unwrap_or_else(|| spec.task.clone()),
        working_dir: dir.to_string_lossy().into_owned(),
        model: spec.model.clone(),
        timeout_secs: spec.timeout_secs,
        idle_timeout_secs: Some(spec.idle_secs),
        ..Default::default()
    };
    let agent_result = execute_stream(agent, opts, |e| on_progress(Progress::Agent(e))).await?;
    on_progress(Progress::AgentFinished {
        exit_code: agent_result.exit_code,
        exit_reason: agent_result.exit_reason,
        duration_ms: agent_result.duration_ms,
    });
    append_event(
        &log_path,
        &CapsuleEvent::AgentFinished {
            provider: provider.clone(),
            exit_code: agent_result.exit_code,
            exit_reason: agent_result.exit_reason,
            duration_ms: agent_result.duration_ms,
            ts: Utc::now(),
        },
    )?;

    let limit = detect_limit(&agent_result.stderr, &agent_result.errors);
    let agent_ok = agent_result.exit_reason == ExitReason::Exited
        && limit.is_none()
        && agent_result.exit_code == 0
        && agent_result.errors.is_empty();

    let mut checks = Vec::new();
    let mut review_result = None;
    if !agent_ok {
        on_progress(Progress::Skipped("agent run did not finish cleanly".into()));
    } else {
        // Run every check, so the evidence covers all failures at once.
        for command in &spec.checks {
            on_progress(Progress::CheckStarted(command.clone()));
            let result = run_check(command, dir, Duration::from_secs(spec.check_timeout_secs)).await;
            append_event(&log_path, &CapsuleEvent::CheckResult { result: result.clone(), ts: Utc::now() })?;
            on_progress(Progress::CheckFinished(result.clone()));
            checks.push(result);
        }

        let checks_passed = checks.iter().all(|c| c.passed);
        if let (true, false, Some(reviewer)) = (checks_passed, spec.accept.is_empty(), reviewer) {
            on_progress(Progress::ReviewStarted(reviewer.name().to_string()));
            let diff = collect_diff(dir, base_commit.as_deref());
            let working_dir = dir.to_string_lossy();
            let req = review::ReviewRequest {
                task: &spec.task,
                criteria: &spec.accept,
                diff: &diff,
                checks: &checks,
                working_dir: &working_dir,
                model: spec.reviewer_model.clone(),
                timeout_secs: spec.timeout_secs.min(900),
            };
            let r = review::review(reviewer, req).await;
            append_event(&log_path, &CapsuleEvent::ReviewResult { review: r.clone(), ts: Utc::now() })?;
            review_result = Some(r);
        } else if !checks_passed && !spec.accept.is_empty() {
            on_progress(Progress::Skipped("review skipped: a check failed".into()));
        }
    }

    let verdict = decide(&provider, &agent_result, limit, &checks, review_result.as_ref());
    let duration_ms = started.elapsed().as_millis() as u64;
    append_event(
        &log_path,
        &CapsuleEvent::Verdict { provider: provider.clone(), verdict: verdict.clone(), duration_ms, ts: Utc::now() },
    )?;

    Ok(StageReport {
        run_id: spec.run_id.clone(),
        attempt: spec.attempt,
        provider,
        exit_code: verdict.exit_code(),
        verdict,
        agent_response: agent_result.response,
        agent_exit_code: agent_result.exit_code,
        agent_exit_reason: agent_result.exit_reason,
        checks,
        review: review_result,
        base_commit,
        duration_ms,
        log_path,
    })
}

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).current_dir(dir).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// HEAD commit, or None outside a git repo / before the first commit.
pub fn git_head(dir: &Path) -> Option<String> {
    git(dir, &["rev-parse", "HEAD"]).map(|s| s.trim().to_string())
}

/// Short summary of what changed since `base` (`git diff --stat` plus new untracked files),
/// excluding orc's own `.orc/` directory. Used in handoff briefs.
pub fn diff_stat(dir: &Path, base: Option<&str>) -> String {
    let Some(base) = base else {
        return "(no git history; inspect the files directly)".to_string();
    };
    let mut out = git(dir, &["diff", "--stat", base, "--", ".", ":(exclude).orc"]).unwrap_or_default();
    let untracked = git(dir, &["ls-files", "--others", "--exclude-standard"]).unwrap_or_default();
    for file in untracked.lines().filter(|f| !f.is_empty() && !f.starts_with(".orc/")) {
        out.push_str(&format!(" {file} (new, untracked)\n"));
    }
    if out.trim().is_empty() {
        "(no changes yet)".to_string()
    } else {
        out.trim_end().to_string()
    }
}

/// Everything that changed since `base`: committed and uncommitted changes to
/// tracked files, plus the contents of new untracked files. Truncated to fit a prompt.
pub fn collect_diff(dir: &Path, base: Option<&str>) -> String {
    let Some(base) = base else {
        return "(no git history to diff against; inspect the files in the repository directly)".to_string();
    };
    // orc's own run log lives in .orc/; it is not the agent's work.
    let mut diff = git(dir, &["diff", base, "--", ".", ":(exclude).orc"]).unwrap_or_default();
    let untracked = git(dir, &["ls-files", "--others", "--exclude-standard"]).unwrap_or_default();
    for file in untracked.lines().filter(|f| !f.is_empty() && !f.starts_with(".orc/")) {
        let content = std::fs::read_to_string(dir.join(file))
            .map(|c| c.chars().take(MAX_UNTRACKED_FILE_CHARS).collect::<String>())
            .unwrap_or_else(|_| "(binary or unreadable)".to_string());
        diff.push_str(&format!("\n--- /dev/null\n+++ b/{file} (new, untracked)\n{content}\n"));
    }
    if diff.trim().is_empty() {
        return "(no changes since the run started)".to_string();
    }
    if diff.chars().count() > MAX_DIFF_CHARS {
        let cut: String = diff.chars().take(MAX_DIFF_CHARS).collect();
        return format!("{cut}\n… (diff truncated; read the files directly for the rest)");
    }
    diff
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capsule::read_events;
    use crate::testutil::ShellProvider;

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        std::fs::write(p.join("calc.sh"), "add() { echo $(($1 + $2)); }\n").unwrap();
        std::fs::write(p.join(".gitignore"), ".orc/\n").unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["-c", "user.email=t@t", "-c", "user.name=t", "add", "."],
            vec!["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qm", "init"],
        ] {
            assert!(Command::new("git").args(&args).current_dir(p).status().unwrap().success());
        }
        dir
    }

    fn spec(dir: &Path, checks: &[&str], accept: &[&str]) -> StageSpec {
        StageSpec {
            run_id: "run-test".into(),
            task: "fix add".into(),
            agent_prompt: None,
            attempt: 1,
            handoff_from: None,
            base_commit: None,
            working_dir: dir.to_path_buf(),
            model: None,
            checks: checks.iter().map(|s| s.to_string()).collect(),
            accept: accept.iter().map(|s| s.to_string()).collect(),
            reviewer_model: None,
            timeout_secs: 30,
            idle_secs: 10,
            check_timeout_secs: 10,
        }
    }

    const TEST_ADD: &str = r#". ./calc.sh; test "$(add 2 3)" = 5 || { echo "expected 5, got $(add 2 3)"; exit 1; }"#;

    /// The step 1 "done when": an agent breaks the tests, orc reports fail with the test output.
    #[tokio::test]
    async fn agent_that_breaks_tests_is_fail_with_evidence() {
        let dir = repo();
        let agent = ShellProvider::new(r#"echo 'add() { echo $(($1 - $2)); }' > calc.sh; echo 'I fixed add.'"#);
        let report = run_stage(&spec(dir.path(), &[TEST_ADD], &[]), &agent, None, |_| {}).await.unwrap();

        match &report.verdict {
            Verdict::Fail { reason, evidence } => {
                assert!(reason.starts_with("check failed"));
                assert!(evidence.contains("expected 5, got -1"), "evidence: {evidence}");
            }
            other => panic!("expected fail, got {other:?}"),
        }
        assert_eq!(report.exit_code, 1);
        assert_eq!(report.agent_response, "I fixed add.");

        let events = read_events(&report.log_path).unwrap();
        let kinds: Vec<&str> = events
            .iter()
            .map(|e| match e {
                CapsuleEvent::StageStarted { .. } => "stage_started",
                CapsuleEvent::AgentFinished { .. } => "agent_finished",
                CapsuleEvent::CheckResult { .. } => "check_result",
                CapsuleEvent::ReviewResult { .. } => "review_result",
                CapsuleEvent::Verdict { .. } => "verdict",
                _ => "other",
            })
            .collect();
        assert_eq!(kinds, ["stage_started", "agent_finished", "check_result", "verdict"]);
    }

    #[tokio::test]
    async fn good_work_with_approving_reviewer_is_pass() {
        let dir = repo();
        let agent = ShellProvider::new("echo '# Calc' > README.md; echo done");
        let reviewer = ShellProvider::new(r#"case "$ORC_MESSAGE" in *"README.md (new, untracked)"*) echo '{"results":[{"id":1,"pass":true,"reason":"README added"}]}';; *) echo '{"results":[{"id":1,"pass":false,"reason":"diff missing"}]}';; esac"#);
        let report = run_stage(&spec(dir.path(), &[TEST_ADD], &["README exists"]), &agent, Some(&reviewer), |_| {})
            .await
            .unwrap();
        assert_eq!(report.verdict, Verdict::Pass, "review: {:?}", report.review);
        assert!(report.review.unwrap().results[0].pass);
    }

    #[tokio::test]
    async fn rejecting_reviewer_is_fail() {
        let dir = repo();
        let agent = ShellProvider::new("echo done");
        let reviewer = ShellProvider::new(r#"echo '{"results":[{"id":1,"pass":false,"reason":"no README"}]}'"#);
        let report = run_stage(&spec(dir.path(), &[], &["README exists"]), &agent, Some(&reviewer), |_| {})
            .await
            .unwrap();
        assert!(matches!(&report.verdict, Verdict::Fail { evidence, .. } if evidence.contains("no README")));
    }

    #[tokio::test]
    async fn limited_agent_skips_checks() {
        let dir = repo();
        let agent = ShellProvider::new("echo 'ERROR: You have hit your usage limit.'; exit 1");
        let mut skipped = false;
        let report = run_stage(&spec(dir.path(), &["exit 1"], &[]), &agent, None, |p| {
            if matches!(p, Progress::Skipped(_)) {
                skipped = true;
            }
        })
        .await
        .unwrap();
        assert!(matches!(report.verdict, Verdict::Limit { .. }));
        assert_eq!(report.exit_code, 3);
        assert!(report.checks.is_empty());
        assert!(skipped);
    }

    #[tokio::test]
    async fn silent_agent_is_stuck() {
        let dir = repo();
        let agent = ShellProvider::new("sleep 30");
        let mut s = spec(dir.path(), &[], &[]);
        s.idle_secs = 1;
        let report = run_stage(&s, &agent, None, |_| {}).await.unwrap();
        assert!(matches!(report.verdict, Verdict::Stuck { .. }));
        assert_eq!(report.exit_code, 2);
    }

    #[tokio::test]
    async fn accept_without_reviewer_is_an_error() {
        let dir = repo();
        let agent = ShellProvider::new("echo hi");
        assert!(run_stage(&spec(dir.path(), &[], &["x"]), &agent, None, |_| {}).await.is_err());
    }

    #[test]
    fn diff_includes_commits_edits_and_untracked_files() {
        let dir = repo();
        let p = dir.path();
        let base = git_head(p).unwrap();
        std::fs::write(p.join("calc.sh"), "add() { echo 0; }\n").unwrap();
        std::fs::write(p.join("NEW.md"), "hello new file").unwrap();
        let diff = collect_diff(p, Some(&base));
        assert!(diff.contains("-add() { echo $(($1 + $2)); }"));
        assert!(diff.contains("+add() { echo 0; }"));
        assert!(diff.contains("NEW.md (new, untracked)"));
        assert!(diff.contains("hello new file"));
        assert!(collect_diff(p, None).contains("no git history"));
    }

    #[test]
    fn diff_stat_lists_changed_and_new_files() {
        let dir = repo();
        let p = dir.path();
        let base = git_head(p).unwrap();
        assert_eq!(diff_stat(p, Some(&base)), "(no changes yet)");
        std::fs::write(p.join("calc.sh"), "add() { echo 0; }\n").unwrap();
        std::fs::write(p.join("NEW.md"), "x").unwrap();
        let stat = diff_stat(p, Some(&base));
        assert!(stat.contains("calc.sh"), "{stat}");
        assert!(stat.contains("NEW.md (new, untracked)"), "{stat}");
    }

    #[test]
    fn diff_leaves_out_orc_run_log() {
        let dir = repo();
        let p = dir.path();
        std::fs::remove_file(p.join(".gitignore")).unwrap(); // a repo that doesn't ignore .orc/
        let base = git_head(p).unwrap();
        std::fs::create_dir_all(p.join(".orc/runs/x")).unwrap();
        std::fs::write(p.join(".orc/runs/x/events.jsonl"), "{}").unwrap();
        let diff = collect_diff(p, Some(&base));
        assert!(!diff.contains("events.jsonl"), "diff: {diff}");
    }
}
