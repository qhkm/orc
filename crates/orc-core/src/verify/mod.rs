//! Step 1 of the orc loop: decide whether a stage is actually done.
//!
//! A stage runs one agent, then orc (not the agent) checks the result:
//! shell checks (`--check`), a reviewer agent for plain-language criteria
//! (`--accept`), and the agent run itself (stuck, rate-limited, failed).
//! See `docs/plans/2026-11-08-verify-step-plan.md`.

pub mod check;
pub mod limit;
pub mod review;
pub mod stage;
pub mod verdict;

pub use check::{run_check, CheckResult};
pub use limit::detect_limit;
pub use review::{CriterionResult, Review};
pub use stage::{run_stage, Progress, StageReport, StageSpec};
pub use verdict::{decide, Verdict};

/// Keep the last `max_lines` lines of `s`, capped at `max_chars` characters.
pub(crate) fn tail(s: &str, max_lines: usize, max_chars: usize) -> String {
    let lines: Vec<&str> = s.trim_end().lines().collect();
    let start = lines.len().saturating_sub(max_lines);
    let joined = lines[start..].join("\n");
    let count = joined.chars().count();
    if count <= max_chars {
        return joined;
    }
    let skip = count - max_chars;
    format!("…{}", joined.chars().skip(skip).collect::<String>())
}

#[cfg(test)]
mod tests {
    use super::tail;

    #[test]
    fn tail_keeps_last_lines_and_caps_chars() {
        assert_eq!(tail("a\nb\nc\n", 2, 100), "b\nc");
        assert_eq!(tail("abcdef", 10, 3), "…def");
        assert_eq!(tail("", 5, 5), "");
    }
}
