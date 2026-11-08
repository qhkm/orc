use regex::Regex;
use std::sync::OnceLock;

/// Rate-limit / quota wording seen from agent CLIs (Codex, Claude Code, Gemini, OpenAI/Anthropic APIs).
/// A bare `429` is not enough: stack traces contain line numbers like `oauth2client.js:429:18`.
const LIMIT_PATTERN: &str = r"(?i)(usage limit|rate[ _-]?limit|quota|too many requests|(status|code|error|http)\W{0,3}429\b|credit balance is too low|limit reached|hit your limit|resource[_ ]exhausted)";

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
            "Request failed with status code 429",
            "Error code: 429 - {'error': {'type': 'rate_limit_reached_error'}}",
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

    /// Real gemini-cli 0.1.4 stderr (SUBSCRIPTION_REQUIRED) on 2026-11-08: an auth/licence
    /// failure, not a rate limit, despite the line number 429 in its stack trace.
    #[test]
    fn stack_trace_line_numbers_are_not_rate_limits() {
        let stderr = "    at async OAuth2Client.requestAsync (/Users/x/.volta/tools/image/packages/@google/gemini-cli/node_modules/google-auth-library/build/src/auth/oauth2client.js:429:18)\n        \"reason\": \"SUBSCRIPTION_REQUIRED\",";
        assert_eq!(detect_limit(stderr, &[]), None);
    }
}
