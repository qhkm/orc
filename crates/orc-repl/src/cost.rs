/// Estimate cost in USD from provider/model and approximate token usage.
pub fn estimate_cost(provider: &str, model: Option<&str>, tokens_in: u64, tokens_out: u64) -> f64 {
    let (price_in_per_m, price_out_per_m) = match (provider, model) {
        ("claude", Some("opus")) => (15.0, 75.0),
        ("claude", Some("sonnet")) => (3.0, 15.0),
        ("claude", Some("haiku")) => (0.25, 1.25),
        ("claude", _) => (3.0, 15.0),
        ("codex", _) => (2.0, 8.0),
        _ => (3.0, 15.0),
    };

    let input_cost = (tokens_in as f64 / 1_000_000.0) * price_in_per_m;
    let output_cost = (tokens_out as f64 / 1_000_000.0) * price_out_per_m;
    input_cost + output_cost
}

pub fn format_cost(cost: f64) -> String {
    format!("${:.2}", cost)
}

#[derive(Debug, Clone)]
pub struct CostTracker {
    spent: f64,
    budget: Option<f64>,
}

impl CostTracker {
    pub fn new() -> Self {
        Self {
            spent: 0.0,
            budget: None,
        }
    }

    pub fn add(&mut self, cost: f64) {
        self.spent += cost;
    }

    pub fn total(&self) -> f64 {
        self.spent
    }

    pub fn set_budget(&mut self, budget: Option<f64>) {
        self.budget = budget;
    }

    pub fn budget(&self) -> Option<f64> {
        self.budget
    }

    pub fn over_budget(&self) -> bool {
        if let Some(budget) = self.budget {
            self.spent > budget
        } else {
            false
        }
    }
}

impl Default for CostTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_cost_claude_opus() {
        let cost = estimate_cost("claude", Some("opus"), 1000, 500);
        assert!(cost > 0.0, "Cost should be positive");
    }

    #[test]
    fn estimate_cost_codex_default() {
        let cost = estimate_cost("codex", None, 1000, 500);
        assert!(cost > 0.0);
    }

    #[test]
    fn estimate_cost_unknown_model_uses_fallback() {
        let cost = estimate_cost("unknown", None, 1000, 500);
        assert!(cost > 0.0, "Unknown provider should use fallback pricing");
    }

    #[test]
    fn format_cost_display() {
        assert_eq!(format_cost(0.0234), "$0.02");
        assert_eq!(format_cost(1.5), "$1.50");
        assert_eq!(format_cost(0.001), "$0.00");
    }

    #[test]
    fn session_budget_tracking() {
        let mut tracker = CostTracker::new();
        assert_eq!(tracker.total(), 0.0);

        tracker.add(0.05);
        tracker.add(0.03);
        assert!((tracker.total() - 0.08).abs() < 0.001);

        tracker.set_budget(Some(0.10));
        assert!(!tracker.over_budget());

        tracker.add(0.05);
        assert!(tracker.over_budget());
    }
}
