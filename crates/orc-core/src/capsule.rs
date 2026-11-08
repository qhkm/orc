//! Append-only per-run event log at `<repo>/.orc/runs/<id>/events.jsonl`.
//! Shared by the REPL and `orc run`; step 3 (routing) learns from these records.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use crate::invoke::ExitReason;
use crate::verify::{CheckResult, Review, Verdict};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CapsuleEvent {
    Classify {
        input: String,
        result: String,
        provider: String,
        model: Option<String>,
        ts: DateTime<Utc>,
    },
    InvokeStart {
        provider: String,
        model: Option<String>,
        tokens_in: u64,
        ts: DateTime<Utc>,
    },
    InvokeEnd {
        provider: String,
        tokens_in: u64,
        tokens_out: u64,
        cost_usd: f64,
        duration_ms: u64,
        ts: DateTime<Utc>,
    },
    SlashCommand {
        command: String,
        ts: DateTime<Utc>,
    },
    UserMessage {
        content: String,
        ts: DateTime<Utc>,
    },
    /// `orc run`: a stage began.
    StageStarted {
        provider: String,
        model: Option<String>,
        task: String,
        /// HEAD before the agent ran; the review diff is taken against it.
        base_commit: Option<String>,
        checks: Vec<String>,
        accept: Vec<String>,
        /// 1-based attempt number within the run (0 in logs written before handoff existed).
        #[serde(default)]
        attempt: u32,
        #[serde(default)]
        handoff_from: Option<String>,
        ts: DateTime<Utc>,
    },
    /// `orc run`: a stage did not pass and orc handed the task to another attempt.
    Handoff {
        from: String,
        to: String,
        /// The attempt number that `to` will run.
        attempt: u32,
        /// The verdict that triggered the handoff, e.g. "limit: codex: You've hit your usage limit".
        reason: String,
        ts: DateTime<Utc>,
    },
    /// `orc run`: the agent process finished or was killed.
    AgentFinished {
        provider: String,
        exit_code: i32,
        exit_reason: ExitReason,
        duration_ms: u64,
        ts: DateTime<Utc>,
    },
    CheckResult {
        result: CheckResult,
        ts: DateTime<Utc>,
    },
    ReviewResult {
        review: Review,
        ts: DateTime<Utc>,
    },
    /// `orc run`: the final verdict for the stage.
    Verdict {
        provider: String,
        verdict: Verdict,
        duration_ms: u64,
        ts: DateTime<Utc>,
    },
}

/// Return the per-session capsule event log path.
pub fn capsule_path(project_root: &Path, session_id: &str) -> PathBuf {
    project_root
        .join(".orc")
        .join("runs")
        .join(session_id)
        .join("events.jsonl")
}

/// Append one event to an events.jsonl log.
pub fn append_event(path: &Path, event: &CapsuleEvent) -> Result<()> {
    use std::io::Write;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let line = serde_json::to_string(event)?;
    writeln!(file, "{}", line)?;
    Ok(())
}

/// Read events from an events.jsonl log.
pub fn read_events(path: &Path) -> Result<Vec<CapsuleEvent>> {
    use std::io::BufRead;

    let file = std::fs::File::open(path)?;
    let reader = std::io::BufReader::new(file);
    let mut events = Vec::new();

    for line in reader.lines() {
        let line = line?;
        if let Ok(event) = serde_json::from_str::<CapsuleEvent>(&line) {
            events.push(event);
        }
    }

    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn append_and_read_events() {
        let dir = TempDir::new().unwrap();
        let session_id = "test-session-001";
        let log_path = capsule_path(dir.path(), session_id);

        let event1 = CapsuleEvent::Classify {
            input: "add auth".to_string(),
            result: "implement".to_string(),
            provider: "codex".to_string(),
            model: None,
            ts: chrono::Utc::now(),
        };

        let event2 = CapsuleEvent::InvokeEnd {
            provider: "codex".to_string(),
            tokens_in: 340,
            tokens_out: 900,
            cost_usd: 0.02,
            duration_ms: 8300,
            ts: chrono::Utc::now(),
        };

        append_event(&log_path, &event1).unwrap();
        append_event(&log_path, &event2).unwrap();

        let events = read_events(&log_path).unwrap();
        assert_eq!(events.len(), 2);
        assert!(
            matches!(&events[0], CapsuleEvent::Classify { result, .. } if result == "implement")
        );
        assert!(matches!(
            &events[1],
            CapsuleEvent::InvokeEnd {
                tokens_out: 900,
                ..
            }
        ));
    }

    #[test]
    fn capsule_path_includes_session_id() {
        let base = std::path::Path::new("/tmp/project");
        let path = capsule_path(base, "abc-123");
        assert!(path.to_string_lossy().contains("abc-123"));
        assert!(path.to_string_lossy().contains("events.jsonl"));
    }
}
