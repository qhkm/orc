use regex::Regex;
use std::sync::OnceLock;

/// Rate-limit / quota wording seen from agent CLIs (Codex, Claude Code, Gemini, OpenAI/Anthropic APIs).
const LIMIT_PATTERN: &str = r"(?i)(usage limit|rate[ _-]?limit|quota|too many requests|\b429\b|credit balance is too low|limit reached|hit your limit|resource[_ ]exhausted)";

fn pattern() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(LIMIT_PATTERN).expect("valid limit regex"))
}

/// Find a rate-limit or quota message in a provider's stderr or error events.
///
/// Only these channels are scanned, never the agent's normal output, so an
/// agent that writes about rate limiting (e.g. implements a rate limiter)
/// is not mistaken for one that hit a limit.
pub fn detect_limit(stderr: &str, errors: &[String]) -> Option<String> {
    errors
        .iter()
        .flat_map(|e| e.lines())
        .chain(stderr.lines())
        .map(str::trim)
        .find(|line| pattern().is_match(line))
        .map(|line| line.chars().take(300).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_real_codex_usage_limit() {
        let errors = vec!["You’ve hit your usage limit. Upgrade to Pro (https://chatgpt.com/explore/pro), visit https://chatgpt.com/settings/usage to purchase more credits or try again at Nov 14th, 2026 3:22 PM.".to_string()];
        let found = detect_limit("", &errors).unwrap();
        assert!(found.starts_with("You’ve hit your usage limit"));
    }

    #[test]
    fn detects_common_limit_messages() {
        for msg in [
            "Claude AI usage limit reached|1762700000",
            "5-hour limit reached ∙ resets 3pm",
            "Error: 429 Too Many Requests",
            "RESOURCE_EXHAUSTED: Quota exceeded for quota metric",
            "rate_limit_error: Number of request tokens has exceeded your per-minute rate limit",
            "Your credit balance is too low to access the Anthropic API",
        ] {
            assert!(detect_limit(msg, &[]).is_some(), "missed: {msg}");
        }
    }

    #[test]
    fn returns_the_matching_line_from_stderr() {
        let stderr = "warming up\nERROR rate limited, retry later\nbye";
        assert_eq!(detect_limit(stderr, &[]).as_deref(), Some("ERROR rate limited, retry later"));
    }

    #[test]
    fn ignores_ordinary_failures() {
        assert!(detect_limit("error[E0425]: cannot find value `x`\ntest result: FAILED", &["error_max_turns".into()]).is_none());
        assert!(detect_limit("", &[]).is_none());
        assert!(detect_limit("listening on port 4290", &[]).is_none());
    }
}
