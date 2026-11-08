//! Step 3 of the orc loop: decide who does a task, from how agents did before.
//! See `docs/plans/2026-11-08-route-step-plan.md`.

use chrono::{DateTime, Duration, Utc};
use regex::Regex;
use serde::Serialize;
use std::sync::OnceLock;
use crate::history::{last_limited_at, stats_for, AttemptRecord};

/// Providers whose latest attempt hit a rate limit within this window go last.
pub const LIMIT_COOLDOWN_MINUTES: i64 = 60;

pub const CATEGORIES: &[&str] = &["debug", "implement", "review", "plan", "question"];

fn re(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).expect("valid category regex"))
}

/// Guess a task's category from its wording. No model call, so it is free and instant.
pub fn categorize(task: &str) -> &'static str {
    static DEBUG: OnceLock<Regex> = OnceLock::new();
    static REVIEW: OnceLock<Regex> = OnceLock::new();
    static PLAN: OnceLock<Regex> = OnceLock::new();
    static QUESTION: OnceLock<Regex> = OnceLock::new();
    let t = task.to_lowercase();
    if re(&DEBUG, r"\b(fix|fixes|fixing|bug|bugs|error|errors|failing|fails|failure|broken|crash|crashes|debug|regression|wrong)\b").is_match(&t) {
        "debug"
    } else if re(&REVIEW, r"\b(review|audit)\b").is_match(&t) {
        "review"
    } else if re(&PLAN, r"\b(plan|design|architect|architecture|spec|proposal|roadmap)\b").is_match(&t) {
        "plan"
    } else if re(&QUESTION, r"^\s*(what|why|how|where|which|explain|does|is|can|should)\b").is_match(&t) {
        "question"
    } else {
        "implement"
    }
}

/// One provider's place in the route, and why.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Ranked {
    pub provider: String,
    pub score: f64,
    pub passes: u32,
    /// Attempts in this category that count (limits excluded).
    pub scored: u32,
    /// The routing table's choice for this category.
    pub configured: bool,
    /// Rate-limited recently; ranked last regardless of score.
    pub limited_at: Option<DateTime<Utc>>,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RoutePlan {
    pub category: String,
    pub ranked: Vec<Ranked>,
}

impl RoutePlan {
    pub fn order(&self) -> Vec<String> {
        self.ranked.iter().map(|r| r.provider.clone()).collect()
    }

    /// One line, e.g. `route (debug): opencode — 3/3 passed · then claude — 1/2 passed · codex — rate-limited 12m ago`.
    /// Listed in hand-off order. A trailing run of providers with nothing to say (no history,
    /// not configured, not limited) is folded into a count, so the line stays short without
    /// misstating the order.
    pub fn explain(&self) -> String {
        let Some((first, rest)) = self.ranked.split_first() else {
            return format!("route ({}): no provider", self.category);
        };
        let nothing_to_say = |r: &Ranked| r.scored == 0 && !r.configured && r.limited_at.is_none();
        let fold_from = rest.iter().rposition(|r| !nothing_to_say(r)).map_or(0, |i| i + 1);
        let mut parts: Vec<String> = rest[..fold_from].iter().map(|r| format!("{} — {}", r.provider, r.reason)).collect();
        let unknown: Vec<&str> = rest[fold_from..].iter().map(|r| r.provider.as_str()).collect();
        if !unknown.is_empty() {
            parts.push(format!("{} with no {} history ({})", unknown.len(), self.category, unknown.join(", ")));
        }
        let head = format!("route ({}): {} — {}", self.category, first.provider, first.reason);
        if parts.is_empty() { head } else { format!("{head} · then {}", parts.join(" · ")) }
    }
}

/// Rank `candidates` (installed providers that can run tasks, in default order)
/// for `category`, using `history`.
///
/// Score is the smoothed pass rate `(passes + 1) / (scored + 2)`; the routing
/// table's choice (`configured`) gets a stronger prior `(passes + 2) / (scored + 3)`,
/// so it leads with no history and real results override it. Recently
/// rate-limited providers go last. Ties: the master provider, then candidate order.
pub fn rank(
    category: &str,
    candidates: &[String],
    history: &[AttemptRecord],
    configured: Option<&str>,
    master: &str,
    now: DateTime<Utc>,
) -> RoutePlan {
    let stats = stats_for(history, Some(category));
    let mut ranked: Vec<(usize, Ranked)> = candidates
        .iter()
        .enumerate()
        .map(|(i, provider)| {
            let s = stats.iter().find(|s| &s.provider == provider);
            let passes = s.map_or(0, |s| s.passes);
            let scored = s.map_or(0, |s| s.scored());
            let is_configured = configured == Some(provider.as_str());
            let score = if is_configured {
                (passes as f64 + 2.0) / (scored as f64 + 3.0)
            } else {
                (passes as f64 + 1.0) / (scored as f64 + 2.0)
            };
            let limited_at = last_limited_at(history, provider)
                .filter(|t| now.signed_duration_since(*t) < Duration::minutes(LIMIT_COOLDOWN_MINUTES));

            let reason = if let Some(t) = limited_at {
                format!("rate-limited {}m ago", now.signed_duration_since(t).num_minutes().max(0))
            } else if scored > 0 {
                let mut r = format!("{passes}/{scored} passed");
                if is_configured {
                    r.push_str(", configured default");
                }
                r
            } else if is_configured {
                format!("no {category} history yet; configured default")
            } else {
                format!("no {category} history yet")
            };
            (i, Ranked { provider: provider.clone(), score, passes, scored, configured: is_configured, limited_at, reason })
        })
        .collect();

    ranked.sort_by(|(ia, a), (ib, b)| {
        a.limited_at
            .is_some()
            .cmp(&b.limited_at.is_some())
            .then(b.score.total_cmp(&a.score))
            .then((b.provider == master).cmp(&(a.provider == master)))
            .then(ia.cmp(ib))
    });

    RoutePlan { category: category.to_string(), ranked: ranked.into_iter().map(|(_, r)| r).collect() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::record;

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn categorize_by_wording() {
        assert_eq!(categorize("The add function in calc.sh is wrong. Fix it."), "debug");
        assert_eq!(categorize("Tests are failing on CI"), "debug");
        assert_eq!(categorize("Review the auth module for security issues"), "review");
        assert_eq!(categorize("Design the database schema for orders"), "plan");
        assert_eq!(categorize("How does the relay pick the next provider?"), "question");
        assert_eq!(categorize("Add a README documenting calc.sh"), "implement");
        assert_eq!(categorize("Create a multiply function"), "implement");
        // "prefix" must not count as "fix"
        assert_eq!(categorize("Add a prefix option"), "implement");
    }

    #[test]
    fn cold_start_follows_the_routing_table_then_master() {
        let plan = rank("debug", &names(&["claude", "codex", "opencode"]), &[], Some("codex"), "claude", Utc::now());
        assert_eq!(plan.order(), ["codex", "claude", "opencode"]);
        assert_eq!(plan.ranked[0].reason, "no debug history yet; configured default");
        assert_eq!(plan.ranked[1].reason, "no debug history yet");
    }

    /// The step 3 "done when": history changes the choice.
    #[test]
    fn history_overrides_the_configured_default() {
        let history = vec![
            record("codex", "debug", "fail", 50),
            record("codex", "debug", "fail", 40),
            record("opencode", "debug", "pass", 30),
            record("opencode", "debug", "pass", 20),
            record("opencode", "debug", "pass", 10),
        ];
        let plan = rank("debug", &names(&["claude", "codex", "opencode"]), &history, Some("codex"), "claude", Utc::now());
        assert_eq!(plan.order(), ["opencode", "claude", "codex"]);
        assert_eq!(plan.ranked[0].reason, "3/3 passed");
        assert_eq!(plan.ranked[2].reason, "0/2 passed, configured default");
        assert_eq!(
            plan.explain(),
            "route (debug): opencode — 3/3 passed · then claude — no debug history yet · codex — 0/2 passed, configured default"
        );
    }

    #[test]
    fn explain_folds_only_trailing_providers_with_nothing_to_say() {
        let history = vec![record("opencode", "debug", "pass", 5), record("pi", "debug", "fail", 4)];
        let plan = rank("debug", &names(&["opencode", "claude", "pi", "gemini", "kimi"]), &history, None, "claude", Utc::now());
        // Order: opencode (0.67), claude (0.5, master), gemini, kimi (0.5), pi (0.33).
        assert_eq!(plan.order(), ["opencode", "claude", "gemini", "kimi", "pi"]);
        assert_eq!(
            plan.explain(),
            "route (debug): opencode — 1/1 passed · then claude — no debug history yet · gemini — no debug history yet · kimi — no debug history yet · pi — 0/1 passed"
        );

        let history = vec![record("opencode", "debug", "pass", 5)];
        let plan = rank("debug", &names(&["opencode", "claude", "gemini", "kimi"]), &history, None, "claude", Utc::now());
        assert_eq!(plan.explain(), "route (debug): opencode — 1/1 passed · then 3 with no debug history (claude, gemini, kimi)");
    }

    #[test]
    fn history_is_per_category() {
        let history = vec![record("opencode", "implement", "pass", 10), record("opencode", "implement", "pass", 5)];
        let plan = rank("debug", &names(&["claude", "opencode"]), &history, None, "claude", Utc::now());
        assert_eq!(plan.order(), ["claude", "opencode"]);
    }

    #[test]
    fn recently_limited_goes_last_and_limits_do_not_hurt_skill() {
        let history = vec![
            record("codex", "debug", "pass", 300),
            record("codex", "debug", "pass", 200),
            record("codex", "debug", "limit", 12),
        ];
        let plan = rank("debug", &names(&["codex", "claude"]), &history, None, "claude", Utc::now());
        assert_eq!(plan.order(), ["claude", "codex"]);
        assert_eq!(plan.ranked[1].reason, "rate-limited 12m ago");
        // 2 passes out of 2 scored: the limit isn't counted against it.
        assert_eq!((plan.ranked[1].passes, plan.ranked[1].scored), (2, 2));

        // After the cooldown, its record puts it first again.
        let later = Utc::now() + Duration::minutes(LIMIT_COOLDOWN_MINUTES);
        let plan = rank("debug", &names(&["codex", "claude"]), &history, None, "claude", later);
        assert_eq!(plan.order(), ["codex", "claude"]);
    }

    #[test]
    fn one_lucky_pass_does_not_beat_a_long_record() {
        let mut history = vec![record("pi", "debug", "pass", 5)];
        for i in 0..9 {
            history.push(record("claude", "debug", "pass", 100 + i));
        }
        history.push(record("claude", "debug", "fail", 99));
        // pi: (1+1)/(1+2)=0.67; claude: (9+1)/(10+2)=0.83
        let plan = rank("debug", &names(&["pi", "claude"]), &history, None, "x", Utc::now());
        assert_eq!(plan.order(), ["claude", "pi"]);
    }
}
