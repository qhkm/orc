//! Step 2 of the orc loop: keep work moving when a stage does not pass.
//!
//! `run_relay` runs attempts of one task, each verified by `verify::run_stage`.
//! When an attempt does not pass, the next attempt goes to another agent when
//! one is available, carrying the verdict, the evidence and the work so far.
//! See `docs/plans/2026-11-08-handoff-step-plan.md`.

use anyhow::Result;
use chrono::Utc;
use serde::Serialize;
use std::collections::HashSet;
use std::path::PathBuf;
use crate::capsule::{append_event, capsule_path, CapsuleEvent};
use crate::invoke::Provider;
use crate::verify::stage::{diff_stat, git_head};
use crate::verify::{run_stage, Progress, StageReport, StageSpec, Verdict};

/// Progress callbacks while a relay runs.
#[derive(Debug, Clone)]
pub enum RelayProgress {
    AttemptStarted { attempt: u32, max_attempts: u32, provider: String },
    Stage(Progress),
    /// The attempt finished with this verdict.
    AttemptFinished { attempt: u32, provider: String, verdict: Verdict },
    Handoff { from: String, to: String, reason: String },
    /// No further attempt will run, and why (not sent on pass).
    Stopped(String),
}

#[derive(Debug, Clone, Serialize)]
pub struct RelayReport {
    pub run_id: String,
    /// The final attempt's verdict.
    pub verdict: Verdict,
    pub exit_code: i32,
    pub attempts: Vec<StageReport>,
    /// Why the relay stopped without a pass; None on pass.
    pub stop_reason: Option<String>,
    pub log_path: PathBuf,
}

/// One-line summary of a verdict, used in handoff reasons and briefs.
pub fn verdict_summary(verdict: &Verdict) -> String {
    match verdict {
        Verdict::Pass => "pass".to_string(),
        Verdict::Fail { reason, .. } => format!("fail: {reason}"),
        Verdict::Stuck { reason, after_secs } => format!("stuck: {reason} (after {after_secs}s)"),
        Verdict::Limit { provider, message } => format!("limit: {provider}: {message}"),
    }
}

/// Pick the provider for the next attempt, or None to stop.
///
/// Candidates are tried round-robin starting after `current`, skipping any
/// provider that hit a rate limit during this run (`limited`).
/// - pass: stop.
/// - limit / stuck: another provider; none left → stop.
/// - fail: another provider; if there is none, the same one again (with the failure as feedback).
pub fn next_provider(verdict: &Verdict, current: usize, count: usize, limited: &HashSet<usize>) -> Option<usize> {
    if matches!(verdict, Verdict::Pass) || count == 0 {
        return None;
    }
    let other = (1..count).map(|k| (current + k) % count).find(|i| !limited.contains(i));
    match verdict {
        Verdict::Fail { .. } => other.or((!limited.contains(&current)).then_some(current)),
        _ => other,
    }
}

/// The prompt for the next attempt: the original task plus what happened so far.
pub fn build_handoff_prompt(task: &str, previous: &StageReport, changes: &str, checks: &[String], accept: &[String]) -> String {
    let evidence = match &previous.verdict {
        Verdict::Fail { evidence, .. } => evidence.clone(),
        Verdict::Limit { message, .. } => message.clone(),
        Verdict::Stuck { reason, .. } => reason.clone(),
        Verdict::Pass => String::new(),
    };
    let mut prompt = format!("{task}\n\n---\n\n## Handoff from orc\n\n");
    prompt.push_str(&format!(
        "This task was already attempted. Attempt {} by {} did not pass orc's verification.\n\n**Verdict:** {}\n",
        previous.attempt,
        previous.provider,
        verdict_summary(&previous.verdict)
    ));
    let evidence = crate::verify::tail(&evidence, 40, 3000);
    if !evidence.trim().is_empty() {
        prompt.push_str(&format!("\n**Evidence:**\n\n```\n{evidence}\n```\n"));
    }
    let said = crate::verify::tail(&previous.agent_response, 20, 1500);
    if !said.trim().is_empty() {
        prompt.push_str(&format!("\n**What {} said it did:**\n\n{said}\n", previous.provider));
    }
    prompt.push_str(&format!(
        "\n**Already changed in the repository since the task started:**\n\n```\n{changes}\n```\n"
    ));
    if !checks.is_empty() || !accept.is_empty() {
        prompt.push_str("\n**To pass, the work must satisfy:**\n\n");
        for c in checks {
            prompt.push_str(&format!("- `{c}` exits 0\n"));
        }
        for a in accept {
            prompt.push_str(&format!("- {a}\n"));
        }
    }
    prompt.push_str(
        "\nContinue from the current state of the repository. Keep the previous changes that are \
         correct and fix what is wrong; do not start over unless they are clearly wrong.\n",
    );
    prompt
}

/// Run `spec.task` until an attempt passes, handing off between `providers`
/// (in order; `providers[0]` goes first) for at most `max_attempts` attempts.
pub async fn run_relay(
    spec: &StageSpec,
    max_attempts: u32,
    providers: &[&dyn Provider],
    reviewer: Option<&dyn Provider>,
    mut on_progress: impl FnMut(RelayProgress) + Send,
) -> Result<RelayReport> {
    anyhow::ensure!(!providers.is_empty(), "no provider to run the task");
    anyhow::ensure!(max_attempts >= 1, "--max-attempts must be at least 1");
    let dir = spec.working_dir.as_path();
    let log_path = capsule_path(dir, &spec.run_id);
    // Pin one base for the whole run, so later reviews see every attempt's changes.
    let base_commit = spec.base_commit.clone().or_else(|| git_head(dir));

    let mut attempts: Vec<StageReport> = Vec::new();
    let mut limited = HashSet::new();
    let mut current = 0usize;
    let mut prompt: Option<String> = spec.agent_prompt.clone();
    let mut handoff_from: Option<String> = None;
    let mut stop_reason = None;

    for attempt in 1..=max_attempts {
        let agent = providers[current];
        on_progress(RelayProgress::AttemptStarted { attempt, max_attempts, provider: agent.name().to_string() });
        let stage = StageSpec {
            attempt,
            agent_prompt: prompt.clone(),
            handoff_from: handoff_from.clone(),
            base_commit: base_commit.clone(),
            // Model names are provider-specific; `spec.model` belongs to the first provider.
            model: if current == 0 { spec.model.clone() } else { None },
            ..spec.clone()
        };
        let report = run_stage(&stage, agent, reviewer, |p| on_progress(RelayProgress::Stage(p))).await?;
        on_progress(RelayProgress::AttemptFinished {
            attempt,
            provider: report.provider.clone(),
            verdict: report.verdict.clone(),
        });
        if matches!(report.verdict, Verdict::Limit { .. }) {
            limited.insert(current);
        }
        let passed = matches!(report.verdict, Verdict::Pass);
        attempts.push(report);
        if passed {
            break;
        }

        let last = attempts.last().expect("just pushed");
        if attempt == max_attempts {
            stop_reason = Some(format!("reached --max-attempts ({max_attempts})"));
            break;
        }
        let Some(next) = next_provider(&last.verdict, current, providers.len(), &limited) else {
            stop_reason = Some(match last.verdict {
                Verdict::Limit { .. } => "every provider hit a rate limit".to_string(),
                _ => "no other provider to hand off to".to_string(),
            });
            break;
        };

        let from = last.provider.clone();
        let to = providers[next].name().to_string();
        let reason = verdict_summary(&last.verdict);
        append_event(
            &log_path,
            &CapsuleEvent::Handoff { from: from.clone(), to: to.clone(), attempt: attempt + 1, reason: reason.clone(), ts: Utc::now() },
        )?;
        on_progress(RelayProgress::Handoff { from: from.clone(), to, reason });

        let changes = diff_stat(dir, base_commit.as_deref());
        prompt = Some(build_handoff_prompt(&spec.task, last, &changes, &spec.checks, &spec.accept));
        handoff_from = Some(from);
        current = next;
    }

    if let Some(reason) = &stop_reason {
        on_progress(RelayProgress::Stopped(reason.clone()));
    }
    let verdict = attempts.last().expect("at least one attempt").verdict.clone();
    Ok(RelayReport {
        run_id: spec.run_id.clone(),
        exit_code: verdict.exit_code(),
        verdict,
        attempts,
        stop_reason,
        log_path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capsule::read_events;
    use crate::invoke::ExitReason;
    use crate::testutil::ShellProvider;
    use std::path::Path;
    use std::process::Command;

    fn fail() -> Verdict {
        Verdict::Fail { reason: "check failed: ./test.sh".into(), evidence: "expected 5, got -1".into() }
    }
    fn limit() -> Verdict {
        Verdict::Limit { provider: "codex".into(), message: "usage limit".into() }
    }
    fn stuck() -> Verdict {
        Verdict::Stuck { reason: "silent".into(), after_secs: 300 }
    }

    #[test]
    fn policy_pass_stops() {
        assert_eq!(next_provider(&Verdict::Pass, 0, 3, &HashSet::new()), None);
    }

    #[test]
    fn policy_moves_round_robin_to_another_provider() {
        let none = HashSet::new();
        for v in [fail(), limit(), stuck()] {
            assert_eq!(next_provider(&v, 0, 3, &none), Some(1), "{v:?}");
            assert_eq!(next_provider(&v, 2, 3, &none), Some(0), "{v:?}");
        }
    }

    #[test]
    fn policy_skips_limited_providers() {
        let limited: HashSet<usize> = [0, 1].into();
        assert_eq!(next_provider(&limit(), 1, 3, &limited), Some(2));
        let all: HashSet<usize> = [0, 1, 2].into();
        assert_eq!(next_provider(&limit(), 2, 3, &all), None);
    }

    #[test]
    fn policy_single_provider_retries_only_on_fail() {
        let none = HashSet::new();
        assert_eq!(next_provider(&fail(), 0, 1, &none), Some(0));
        assert_eq!(next_provider(&stuck(), 0, 1, &none), None);
        assert_eq!(next_provider(&limit(), 0, 1, &[0].into()), None);
    }

    #[test]
    fn policy_fail_after_others_limited_retries_same() {
        // codex hit a limit, claude failed: claude tries again.
        assert_eq!(next_provider(&fail(), 1, 2, &[0].into()), Some(1));
    }

    fn report(provider: &str, verdict: Verdict) -> StageReport {
        StageReport {
            run_id: "r".into(),
            attempt: 1,
            provider: provider.into(),
            exit_code: verdict.exit_code(),
            verdict,
            agent_response: "I changed add to subtract.".into(),
            agent_exit_code: 0,
            agent_exit_reason: ExitReason::Exited,
            checks: vec![],
            review: None,
            base_commit: None,
            duration_ms: 0,
            log_path: PathBuf::new(),
        }
    }

    #[test]
    fn brief_carries_verdict_evidence_changes_and_targets() {
        let p = build_handoff_prompt(
            "Fix add",
            &report("codex", fail()),
            " calc.sh | 2 +-",
            &["./test.sh".into()],
            &["has a README".into()],
        );
        assert!(p.starts_with("Fix add\n"));
        assert!(p.contains("Attempt 1 by codex did not pass"));
        assert!(p.contains("fail: check failed: ./test.sh"));
        assert!(p.contains("expected 5, got -1"));
        assert!(p.contains("I changed add to subtract."));
        assert!(p.contains("calc.sh | 2 +-"));
        assert!(p.contains("- `./test.sh` exits 0"));
        assert!(p.contains("- has a README"));
        assert!(p.contains("Continue from the current state"));
    }

    // --- end to end with shell providers ---

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        std::fs::write(p.join("calc.sh"), "add() { echo $(($1 - $2)); }\n").unwrap(); // broken
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

    fn spec(dir: &Path, accept: &[&str]) -> StageSpec {
        StageSpec {
            run_id: "run-relay".into(),
            task: "Fix add in calc.sh".into(),
            agent_prompt: None,
            attempt: 1,
            handoff_from: None,
            base_commit: None,
            working_dir: dir.to_path_buf(),
            model: None,
            checks: vec![r#". ./calc.sh; test "$(add 2 3)" = 5 || { echo "expected 5, got $(add 2 3)"; exit 1; }"#.into()],
            accept: accept.iter().map(|s| s.to_string()).collect(),
            reviewer_model: None,
            timeout_secs: 30,
            idle_secs: 10,
            check_timeout_secs: 10,
        }
    }

    const FIX: &str = r#"echo 'add() { echo $(($1 + $2)); }' > calc.sh; echo fixed"#;
    const LIMITED: &str = "echo 'ERROR: You have hit your usage limit.'; exit 1";

    /// The step 2 "done when": a rate-limited agent is followed by one that finishes.
    #[tokio::test]
    async fn limited_agent_hands_off_and_next_agent_finishes() {
        let dir = repo();
        let codex = ShellProvider::named("codex", LIMITED);
        // Only fixes the file if it was told about the earlier attempt.
        let claude = ShellProvider::named(
            "claude",
            &format!(r#"case "$ORC_MESSAGE" in *"Attempt 1 by codex did not pass"*"usage limit"*) {FIX};; *) echo "no brief";; esac"#),
        );
        let mut handoffs = vec![];
        let report = run_relay(&spec(dir.path(), &[]), 3, &[&codex, &claude], None, |p| {
            if let RelayProgress::Handoff { from, to, .. } = p {
                handoffs.push((from, to));
            }
        })
        .await
        .unwrap();

        assert_eq!(report.verdict, Verdict::Pass, "attempts: {:#?}", report.attempts);
        assert_eq!(report.attempts.len(), 2);
        assert_eq!(report.attempts[1].attempt, 2);
        assert_eq!(report.stop_reason, None);
        assert_eq!(handoffs, vec![("codex".to_string(), "claude".to_string())]);

        let events = read_events(&report.log_path).unwrap();
        let handoff = events.iter().find_map(|e| match e {
            CapsuleEvent::Handoff { from, to, attempt, reason, .. } => Some((from, to, *attempt, reason)),
            _ => None,
        });
        let (from, to, attempt, reason) = handoff.expect("handoff event logged");
        assert_eq!((from.as_str(), to.as_str(), attempt), ("codex", "claude", 2));
        assert!(reason.starts_with("limit: codex"));
        let second_start = events.iter().filter_map(|e| match e {
            CapsuleEvent::StageStarted { attempt, handoff_from, .. } => Some((*attempt, handoff_from.clone())),
            _ => None,
        });
        assert_eq!(second_start.collect::<Vec<_>>(), vec![(1, None), (2, Some("codex".to_string()))]);
    }

    #[tokio::test]
    async fn single_provider_retries_with_failure_feedback() {
        let dir = repo();
        let agent = ShellProvider::named(
            "claude",
            &format!(r#"case "$ORC_MESSAGE" in *"expected 5, got -1"*) {FIX};; *) echo "looks fine to me";; esac"#),
        );
        let report = run_relay(&spec(dir.path(), &[]), 3, &[&agent], None, |_| {}).await.unwrap();
        assert_eq!(report.verdict, Verdict::Pass);
        assert_eq!(report.attempts.len(), 2);
        assert!(matches!(report.attempts[0].verdict, Verdict::Fail { .. }));
    }

    #[tokio::test]
    async fn stops_when_every_provider_is_limited() {
        let dir = repo();
        let a = ShellProvider::named("codex", LIMITED);
        let b = ShellProvider::named("claude", LIMITED);
        let report = run_relay(&spec(dir.path(), &[]), 5, &[&a, &b], None, |_| {}).await.unwrap();
        assert_eq!(report.attempts.len(), 2);
        assert!(matches!(report.verdict, Verdict::Limit { .. }));
        assert_eq!(report.exit_code, 3);
        assert_eq!(report.stop_reason.as_deref(), Some("every provider hit a rate limit"));
    }

    #[tokio::test]
    async fn stops_at_max_attempts() {
        let dir = repo();
        let a = ShellProvider::named("codex", "echo nope");
        let b = ShellProvider::named("claude", "echo nope");
        let report = run_relay(&spec(dir.path(), &[]), 3, &[&a, &b], None, |_| {}).await.unwrap();
        let who: Vec<&str> = report.attempts.iter().map(|r| r.provider.as_str()).collect();
        assert_eq!(who, ["codex", "claude", "codex"]);
        assert!(matches!(report.verdict, Verdict::Fail { .. }));
        assert_eq!(report.stop_reason.as_deref(), Some("reached --max-attempts (3)"));
    }

    #[tokio::test]
    async fn reviewer_judges_original_task_and_whole_run_diff() {
        let dir = repo();
        // Attempt 1 commits a README (moving HEAD), then hits a limit.
        let a = ShellProvider::named(
            "codex",
            "echo '# Calc' > README.md; git add README.md; git -c user.email=t@t -c user.name=t commit -qm readme; echo 'ERROR: usage limit'; exit 1",
        );
        let b = ShellProvider::named("claude", FIX);
        // Passes only if it sees the original task (not the brief) and attempt 1's committed README.
        let reviewer = ShellProvider::named(
            "reviewer",
            r#"case "$ORC_MESSAGE" in *"Handoff from orc"*) echo '{"results":[{"id":1,"pass":false,"reason":"saw brief"}]}';; *"+# Calc"*) echo '{"results":[{"id":1,"pass":true,"reason":"ok"}]}';; *) echo '{"results":[{"id":1,"pass":false,"reason":"missing README"}]}';; esac"#,
        );
        let report = run_relay(&spec(dir.path(), &["has a README"]), 3, &[&a, &b], Some(&reviewer), |_| {})
            .await
            .unwrap();
        assert_eq!(report.verdict, Verdict::Pass, "review: {:?}", report.attempts.last().unwrap().review);
    }
}
