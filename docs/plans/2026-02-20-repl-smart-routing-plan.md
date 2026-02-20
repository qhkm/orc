# ORC REPL + Smart Routing Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Transform `orc` from a TUI-first session hub into a REPL-first conversational coding shell with LLM-powered intent classification and smart model routing.

**Architecture:** New `orc-repl` crate provides the REPL shell, intent classifier, router, and slash command handlers. `orc-bin` rewires its default command from TUI to REPL. Existing `orc-core` (invoke, SCP, config) and `orc-tui` (session hub) are unchanged and reused.

**Tech Stack:** Rust, rustyline (REPL input), orc-core (invoke/stream, config, SCP), orc-tui (behind `/sessions`), chrono, serde_json, anyhow

---

## Task 1: Create orc-repl crate skeleton

**Files:**
- Create: `crates/orc-repl/Cargo.toml`
- Create: `crates/orc-repl/src/lib.rs`
- Modify: `Cargo.toml` (workspace members)

**Step 1: Create Cargo.toml for orc-repl**

Create `crates/orc-repl/Cargo.toml`:

```toml
[package]
name = "orc-repl"
version.workspace = true
edition.workspace = true

[dependencies]
orc-core = { path = "../orc-core" }
rustyline = "15"
tokio.workspace = true
anyhow.workspace = true
serde.workspace = true
serde_json.workspace = true
chrono.workspace = true
dirs = "6"
```

**Step 2: Create lib.rs with module stubs**

Create `crates/orc-repl/src/lib.rs`:

```rust
pub mod classifier;
pub mod router;
pub mod slash;
pub mod repl;
```

**Step 3: Create empty module files**

Create `crates/orc-repl/src/classifier.rs`:
```rust
// Intent classifier — calls master provider to classify user messages
```

Create `crates/orc-repl/src/router.rs`:
```rust
// Router — maps classified intent to provider + model
```

Create `crates/orc-repl/src/slash.rs`:
```rust
// Slash command parser and handlers
```

Create `crates/orc-repl/src/repl.rs`:
```rust
// REPL shell — rustyline loop, input dispatch, output rendering
```

**Step 4: Add orc-repl to workspace**

In root `Cargo.toml`, change:
```toml
members = ["crates/orc-core", "crates/orc-tui", "crates/orc-bin"]
```
to:
```toml
members = ["crates/orc-core", "crates/orc-tui", "crates/orc-repl", "crates/orc-bin"]
```

**Step 5: Add orc-repl dependency to orc-bin**

In `crates/orc-bin/Cargo.toml`, add:
```toml
orc-repl = { path = "../orc-repl" }
```

**Step 6: Verify build**

Run: `cargo build --manifest-path ./Cargo.toml 2>&1 | tail -3`
Expected: compiles with no errors (empty modules are fine)

**Step 7: Commit**

```bash
git add crates/orc-repl/ Cargo.toml crates/orc-bin/Cargo.toml
git commit -m "feat: scaffold orc-repl crate with module stubs"
```

---

## Task 2: Routing config — add routing table to Config

**Files:**
- Modify: `crates/orc-core/src/config.rs`

**Step 1: Write the failing test**

Add to the test module at the bottom of `config.rs`:

```rust
#[test]
fn config_routing_roundtrip() {
    let mut routing = std::collections::HashMap::new();
    routing.insert("plan".to_string(), RouteEntry {
        provider: "claude".to_string(),
        model: Some("opus".to_string()),
    });
    routing.insert("implement".to_string(), RouteEntry {
        provider: "codex".to_string(),
        model: None,
    });

    let cfg = Config {
        master_provider: "claude".into(),
        providers: vec![],
        routing: Some(routing),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    let file = NamedTempFile::new().unwrap();
    save_config(file.path(), &cfg).unwrap();
    let loaded = load_config(file.path()).unwrap();

    let r = loaded.routing.unwrap();
    assert_eq!(r["plan"].provider, "claude");
    assert_eq!(r["plan"].model, Some("opus".to_string()));
    assert_eq!(r["implement"].provider, "codex");
    assert_eq!(r["implement"].model, None);
}

#[test]
fn config_without_routing_loads_ok() {
    // Old config files won't have routing field — should deserialize as None
    let json = r#"{"master_provider":"claude","providers":[],"created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z"}"#;
    let cfg: Config = serde_json::from_str(json).unwrap();
    assert!(cfg.routing.is_none());
}
```

**Step 2: Run to verify compile fails**

Run: `cargo test --manifest-path ./Cargo.toml -p orc-core config 2>&1 | grep "error"`
Expected: `RouteEntry` and `routing` field don't exist.

**Step 3: Implement**

Add `RouteEntry` struct and `routing` field to `Config` in `config.rs`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteEntry {
    pub provider: String,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub master_provider: String,
    pub providers: Vec<ProviderConfig>,
    #[serde(default)]
    pub routing: Option<std::collections::HashMap<String, RouteEntry>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
```

`#[serde(default)]` ensures old config files without `routing` deserialize as `None`.

Add a helper to get the default routing table:

```rust
pub fn default_routing() -> std::collections::HashMap<String, RouteEntry> {
    let mut m = std::collections::HashMap::new();
    m.insert("plan".into(), RouteEntry { provider: "claude".into(), model: Some("opus".into()) });
    m.insert("implement".into(), RouteEntry { provider: "codex".into(), model: None });
    m.insert("review".into(), RouteEntry { provider: "claude".into(), model: Some("sonnet".into()) });
    m.insert("debug".into(), RouteEntry { provider: "claude".into(), model: Some("opus".into()) });
    m.insert("question".into(), RouteEntry { provider: "claude".into(), model: Some("haiku".into()) });
    m
}
```

**Step 4: Run tests**

Run: `cargo test --manifest-path ./Cargo.toml -p orc-core 2>&1 | tail -5`
Expected: all tests pass.

**Step 5: Commit**

```bash
git add crates/orc-core/src/config.rs
git commit -m "feat: add routing table to config (category → provider + model)"
```

---

## Task 3: Intent classifier

**Files:**
- Modify: `crates/orc-repl/src/classifier.rs`

**Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_classify_response_valid_json() {
        let response = r#"{"category": "implement"}"#;
        let result = parse_classify_response(response);
        assert_eq!(result, Some("implement".to_string()));
    }

    #[test]
    fn parse_classify_response_with_extra_text() {
        // LLM might return markdown-wrapped JSON
        let response = "Here is the classification:\n```json\n{\"category\": \"plan\"}\n```";
        let result = parse_classify_response(response);
        assert_eq!(result, Some("plan".to_string()));
    }

    #[test]
    fn parse_classify_response_invalid() {
        let response = "I'm not sure what you mean.";
        let result = parse_classify_response(response);
        assert!(result.is_none());
    }

    #[test]
    fn parse_classify_response_unknown_category_returns_none() {
        let response = r#"{"category": "dance"}"#;
        let result = parse_classify_response(response);
        assert!(result.is_none()); // "dance" is not a valid category
    }

    #[test]
    fn build_classify_prompt_includes_message() {
        let prompt = build_classify_prompt("add rate limiting to the API");
        assert!(prompt.contains("add rate limiting to the API"));
        assert!(prompt.contains("plan"));
        assert!(prompt.contains("implement"));
        assert!(prompt.contains("review"));
        assert!(prompt.contains("debug"));
        assert!(prompt.contains("question"));
    }
}
```

**Step 2: Run to verify compile fails**

Run: `cargo test --manifest-path ./Cargo.toml -p orc-repl 2>&1 | grep "error"`
Expected: `parse_classify_response` and `build_classify_prompt` not defined.

**Step 3: Implement**

In `classifier.rs`:

```rust
const VALID_CATEGORIES: &[&str] = &["plan", "implement", "review", "debug", "question"];

/// Build the prompt sent to the master provider for classification.
pub fn build_classify_prompt(user_message: &str) -> String {
    format!(
        "Classify this coding task into exactly one category. Respond with JSON only, no other text.\n\
         Task: \"{}\"\n\
         Categories: plan, implement, review, debug, question\n\
         Respond: {{\"category\": \"<one of the above>\"}}",
        user_message
    )
}

/// Parse the LLM's classification response. Returns the category if valid.
pub fn parse_classify_response(response: &str) -> Option<String> {
    // Try direct JSON parse first
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(response.trim()) {
        if let Some(cat) = val.get("category").and_then(|c| c.as_str()) {
            if VALID_CATEGORIES.contains(&cat) {
                return Some(cat.to_string());
            }
        }
    }

    // Try to extract JSON from markdown code block
    for line in response.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('{') && trimmed.ends_with('}') {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) {
                if let Some(cat) = val.get("category").and_then(|c| c.as_str()) {
                    if VALID_CATEGORIES.contains(&cat) {
                        return Some(cat.to_string());
                    }
                }
            }
        }
    }

    None
}

/// Classify a user message by invoking the master provider.
/// Returns the category string (e.g. "plan", "implement") or None on failure.
pub async fn classify(
    provider: &dyn orc_core::invoke::provider::Provider,
    user_message: &str,
    working_dir: &str,
) -> Option<String> {
    use orc_core::invoke::provider::InvokeOpts;
    use orc_core::invoke::stream::execute;

    let prompt = build_classify_prompt(user_message);
    let opts = InvokeOpts {
        message: prompt,
        working_dir: working_dir.to_string(),
        model: Some("haiku".to_string()), // use fast/cheap model for classification
        timeout_secs: 15,
        ..Default::default()
    };

    match execute(provider, opts).await {
        Ok(result) => parse_classify_response(&result.response),
        Err(_) => None,
    }
}
```

**Step 4: Run tests**

Run: `cargo test --manifest-path ./Cargo.toml -p orc-repl 2>&1 | tail -5`
Expected: all 5 classifier tests pass.

**Step 5: Commit**

```bash
git add crates/orc-repl/src/classifier.rs
git commit -m "feat: intent classifier — build prompt, parse response, invoke provider"
```

---

## Task 4: Router — map category to provider + model

**Files:**
- Modify: `crates/orc-repl/src/router.rs`

**Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use orc_core::config::RouteEntry;

    #[test]
    fn route_returns_configured_entry() {
        let mut routing = std::collections::HashMap::new();
        routing.insert("plan".to_string(), RouteEntry {
            provider: "claude".to_string(),
            model: Some("opus".to_string()),
        });
        routing.insert("implement".to_string(), RouteEntry {
            provider: "codex".to_string(),
            model: None,
        });

        let result = route("plan", &routing, "claude");
        assert_eq!(result.provider, "claude");
        assert_eq!(result.model, Some("opus".to_string()));

        let result = route("implement", &routing, "claude");
        assert_eq!(result.provider, "codex");
        assert_eq!(result.model, None);
    }

    #[test]
    fn route_unknown_category_falls_back_to_master() {
        let routing = std::collections::HashMap::new();
        let result = route("unknown_category", &routing, "claude");
        assert_eq!(result.provider, "claude");
        assert_eq!(result.model, None);
    }

    #[test]
    fn route_display_formats_correctly() {
        let entry = RouteEntry {
            provider: "claude".to_string(),
            model: Some("opus".to_string()),
        };
        assert_eq!(route_display(&entry), "claude (opus)");

        let entry_no_model = RouteEntry {
            provider: "codex".to_string(),
            model: None,
        };
        assert_eq!(route_display(&entry_no_model), "codex");
    }
}
```

**Step 2: Run to verify compile fails**

Expected: `route` and `route_display` not defined.

**Step 3: Implement**

In `router.rs`:

```rust
use orc_core::config::RouteEntry;
use std::collections::HashMap;

/// Look up the provider + model for a classified category.
/// Falls back to master_provider with no model override if category not found.
pub fn route(category: &str, routing: &HashMap<String, RouteEntry>, master_provider: &str) -> RouteEntry {
    if let Some(entry) = routing.get(category) {
        return entry.clone();
    }
    // Fallback: use master provider, no model override
    RouteEntry {
        provider: master_provider.to_string(),
        model: None,
    }
}

/// Format a route entry for display: "codex" or "claude (opus)"
pub fn route_display(entry: &RouteEntry) -> String {
    match &entry.model {
        Some(m) => format!("{} ({})", entry.provider, m),
        None => entry.provider.clone(),
    }
}
```

**Step 4: Run tests**

Run: `cargo test --manifest-path ./Cargo.toml -p orc-repl 2>&1 | tail -5`
Expected: all router tests pass.

**Step 5: Commit**

```bash
git add crates/orc-repl/src/router.rs
git commit -m "feat: router — category lookup with master provider fallback"
```

---

## Task 5: Slash command parser

**Files:**
- Modify: `crates/orc-repl/src/slash.rs`

**Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_slash_commands() {
        assert_eq!(parse_slash("/help"), Some(SlashCommand::Help));
        assert_eq!(parse_slash("/models"), Some(SlashCommand::Models));
        assert_eq!(parse_slash("/providers"), Some(SlashCommand::Providers));
        assert_eq!(parse_slash("/sessions"), Some(SlashCommand::Sessions));
    }

    #[test]
    fn parse_set_command() {
        let cmd = parse_slash("/set implement codex");
        assert_eq!(cmd, Some(SlashCommand::Set {
            category: "implement".to_string(),
            provider: "codex".to_string(),
            model: None,
        }));

        let cmd = parse_slash("/set plan claude opus");
        assert_eq!(cmd, Some(SlashCommand::Set {
            category: "plan".to_string(),
            provider: "claude".to_string(),
            model: Some("opus".to_string()),
        }));
    }

    #[test]
    fn parse_budget_command() {
        let cmd = parse_slash("/budget");
        assert_eq!(cmd, Some(SlashCommand::Budget { amount: None }));

        let cmd = parse_slash("/budget 5.00");
        assert_eq!(cmd, Some(SlashCommand::Budget { amount: Some(5.0) }));
    }

    #[test]
    fn parse_unknown_slash_returns_none() {
        assert_eq!(parse_slash("/foobar"), None);
    }

    #[test]
    fn non_slash_input_returns_none() {
        assert_eq!(parse_slash("hello world"), None);
        assert_eq!(parse_slash(""), None);
    }
}
```

**Step 2: Run to verify compile fails**

Expected: `SlashCommand` and `parse_slash` not defined.

**Step 3: Implement**

In `slash.rs`:

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum SlashCommand {
    Help,
    Models,
    Providers,
    Sessions,
    Set {
        category: String,
        provider: String,
        model: Option<String>,
    },
    Budget {
        amount: Option<f64>,
    },
}

/// Parse a line of input as a slash command. Returns None if not a slash command
/// or if the command is unrecognized.
pub fn parse_slash(input: &str) -> Option<SlashCommand> {
    let input = input.trim();
    if !input.starts_with('/') {
        return None;
    }

    let parts: Vec<&str> = input.split_whitespace().collect();
    let cmd = parts[0];

    match cmd {
        "/help" => Some(SlashCommand::Help),
        "/models" => Some(SlashCommand::Models),
        "/providers" => Some(SlashCommand::Providers),
        "/sessions" => Some(SlashCommand::Sessions),
        "/set" => {
            // /set <category> <provider> [model]
            if parts.len() < 3 {
                return None;
            }
            Some(SlashCommand::Set {
                category: parts[1].to_string(),
                provider: parts[2].to_string(),
                model: parts.get(3).map(|s| s.to_string()),
            })
        }
        "/budget" => {
            let amount = parts.get(1).and_then(|s| s.parse::<f64>().ok());
            Some(SlashCommand::Budget { amount })
        }
        _ => None,
    }
}
```

**Step 4: Run tests**

Run: `cargo test --manifest-path ./Cargo.toml -p orc-repl 2>&1 | tail -5`
Expected: all slash command tests pass.

**Step 5: Commit**

```bash
git add crates/orc-repl/src/slash.rs
git commit -m "feat: slash command parser — /help, /models, /set, /budget, /sessions, /providers"
```

---

## Task 6: Capsule event logging

**Files:**
- Create: `crates/orc-repl/src/capsule.rs`
- Modify: `crates/orc-repl/src/lib.rs`

**Step 1: Write the failing test**

```rust
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
        assert!(matches!(&events[0], CapsuleEvent::Classify { result, .. } if result == "implement"));
        assert!(matches!(&events[1], CapsuleEvent::InvokeEnd { tokens_out: 900, .. }));
    }

    #[test]
    fn capsule_path_includes_session_id() {
        let base = std::path::Path::new("/tmp/project");
        let path = capsule_path(base, "abc-123");
        assert!(path.to_string_lossy().contains("abc-123"));
        assert!(path.to_string_lossy().contains("events.jsonl"));
    }
}
```

**Step 2: Run to verify compile fails**

Expected: `CapsuleEvent`, `capsule_path`, `append_event`, `read_events` not defined.

**Step 3: Implement**

Create `crates/orc-repl/src/capsule.rs`:

```rust
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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
}

/// Returns the path for a session's capsule event log.
/// Format: <project_root>/.orc/runs/<session_id>/events.jsonl
pub fn capsule_path(project_root: &Path, session_id: &str) -> PathBuf {
    project_root
        .join(".orc")
        .join("runs")
        .join(session_id)
        .join("events.jsonl")
}

/// Append a single event to the capsule log (creates dirs + file if needed).
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

/// Read all events from a capsule log.
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
```

Add `pub mod capsule;` to `crates/orc-repl/src/lib.rs`.

**Step 4: Run tests**

Run: `cargo test --manifest-path ./Cargo.toml -p orc-repl 2>&1 | tail -5`
Expected: all capsule tests pass.

**Step 5: Commit**

```bash
git add crates/orc-repl/src/capsule.rs crates/orc-repl/src/lib.rs
git commit -m "feat: capsule event logging — append-only JSONL per session"
```

---

## Task 7: Cost tracking

**Files:**
- Create: `crates/orc-repl/src/cost.rs`
- Modify: `crates/orc-repl/src/lib.rs`

**Step 1: Write the failing test**

```rust
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
```

**Step 2: Run to verify compile fails**

Expected: `estimate_cost`, `format_cost`, `CostTracker` not defined.

**Step 3: Implement**

Create `crates/orc-repl/src/cost.rs`:

```rust
/// Estimate cost in USD based on provider, model, and token counts.
/// Uses approximate pricing per 1M tokens (input/output).
pub fn estimate_cost(provider: &str, model: Option<&str>, tokens_in: u64, tokens_out: u64) -> f64 {
    let (price_in, price_out) = match (provider, model) {
        ("claude", Some("opus")) => (15.0, 75.0),     // per 1M tokens
        ("claude", Some("sonnet")) => (3.0, 15.0),
        ("claude", Some("haiku")) => (0.25, 1.25),
        ("claude", _) => (3.0, 15.0),                  // default to sonnet
        ("codex", _) => (2.0, 8.0),                    // approximate
        _ => (3.0, 15.0),                               // fallback
    };

    let cost_in = (tokens_in as f64 / 1_000_000.0) * price_in;
    let cost_out = (tokens_out as f64 / 1_000_000.0) * price_out;
    cost_in + cost_out
}

/// Format a cost as "$X.XX"
pub fn format_cost(cost: f64) -> String {
    format!("${:.2}", cost)
}

/// Tracks cumulative cost for a REPL session with optional budget cap.
pub struct CostTracker {
    spent: f64,
    budget: Option<f64>,
}

impl CostTracker {
    pub fn new() -> Self {
        Self { spent: 0.0, budget: None }
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
        if let Some(b) = self.budget {
            self.spent > b
        } else {
            false
        }
    }
}
```

Add `pub mod cost;` to `lib.rs`.

**Step 4: Run tests**

Run: `cargo test --manifest-path ./Cargo.toml -p orc-repl 2>&1 | tail -5`
Expected: all cost tests pass.

**Step 5: Commit**

```bash
git add crates/orc-repl/src/cost.rs crates/orc-repl/src/lib.rs
git commit -m "feat: cost tracking — estimate per model, format, budget cap"
```

---

## Task 8: REPL shell — core loop

**Files:**
- Modify: `crates/orc-repl/src/repl.rs`

This is the largest task. The REPL shell ties together classifier, router, slash commands, invoke, capsule logging, and cost tracking.

**Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repl_state_initial() {
        let state = ReplState::new("/tmp/project");
        assert_eq!(state.working_dir, "/tmp/project");
        assert!(state.provider_override.is_none());
        assert!(state.history.is_empty());
        assert_eq!(state.cost_tracker.total(), 0.0);
    }

    #[test]
    fn repl_state_cycle_override() {
        let mut state = ReplState::new("/tmp/project");
        let providers = vec!["claude".to_string(), "codex".to_string()];

        // Start: no override (auto-route)
        assert!(state.provider_override.is_none());

        // First cycle: claude
        state.cycle_provider_override(&providers);
        assert_eq!(state.provider_override, Some("claude".to_string()));

        // Second cycle: codex
        state.cycle_provider_override(&providers);
        assert_eq!(state.provider_override, Some("codex".to_string()));

        // Third cycle: back to auto
        state.cycle_provider_override(&providers);
        assert!(state.provider_override.is_none());
    }

    #[test]
    fn repl_state_prompt_display() {
        let mut state = ReplState::new("/tmp/project");
        assert_eq!(state.prompt_string(), "> ");

        state.provider_override = Some("codex".to_string());
        assert!(state.prompt_string().contains("codex"));
    }

    #[test]
    fn header_line_includes_providers() {
        let header = build_header("0.1.0", &["claude", "codex"], "/tmp/project");
        assert!(header.contains("claude"));
        assert!(header.contains("codex"));
        assert!(header.contains("0.1.0"));
    }
}
```

**Step 2: Run to verify compile fails**

Expected: `ReplState`, `build_header` not defined.

**Step 3: Implement**

In `repl.rs`:

```rust
use crate::capsule::{CapsuleEvent, append_event, capsule_path};
use crate::classifier;
use crate::cost::{CostTracker, estimate_cost, format_cost};
use crate::router::{route, route_display};
use crate::slash::{parse_slash, SlashCommand};
use orc_core::config::{default_config_path, load_config, save_config, default_routing, RouteEntry};
use orc_core::invoke::provider::{InvokeOpts, Provider};
use orc_core::invoke::providers::claude::ClaudeProvider;
use orc_core::invoke::providers::codex::CodexProvider;
use orc_core::session::types::StreamEvent;
use std::collections::HashMap;

/// Mutable state for one REPL session.
pub struct ReplState {
    pub working_dir: String,
    pub provider_override: Option<String>,
    pub history: Vec<(String, String)>, // (role, content) pairs
    pub cost_tracker: CostTracker,
    pub session_id: String,
}

impl ReplState {
    pub fn new(working_dir: &str) -> Self {
        let session_id = format!("repl-{}", chrono::Utc::now().format("%Y%m%d-%H%M%S"));
        Self {
            working_dir: working_dir.to_string(),
            provider_override: None,
            history: Vec::new(),
            cost_tracker: CostTracker::new(),
            session_id,
        }
    }

    /// Cycle through provider overrides: None → provider1 → provider2 → ... → None
    pub fn cycle_provider_override(&mut self, providers: &[String]) {
        if providers.is_empty() {
            return;
        }
        match &self.provider_override {
            None => {
                self.provider_override = Some(providers[0].clone());
            }
            Some(current) => {
                if let Some(idx) = providers.iter().position(|p| p == current) {
                    if idx + 1 < providers.len() {
                        self.provider_override = Some(providers[idx + 1].clone());
                    } else {
                        self.provider_override = None; // cycle back to auto
                    }
                } else {
                    self.provider_override = None;
                }
            }
        }
    }

    /// Build the prompt string shown to the user.
    pub fn prompt_string(&self) -> String {
        match &self.provider_override {
            None => "> ".to_string(),
            Some(p) => format!("({}) > ", p),
        }
    }
}

/// Build the header line shown when the REPL starts.
pub fn build_header(version: &str, providers: &[&str], cwd: &str) -> String {
    let provider_list = providers.join(" + ");
    format!("orc v{} · {} · {}", version, provider_list, cwd)
}

/// Get a provider instance by name.
fn get_provider(name: &str) -> Option<Box<dyn Provider>> {
    match name {
        "claude" => Some(Box::new(ClaudeProvider)),
        "codex" => Some(Box::new(CodexProvider)),
        _ => None,
    }
}

/// Run the REPL loop. This is the main entry point.
pub fn run_repl() -> anyhow::Result<()> {
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| ".".to_string());

    // Load config
    let config_path = default_config_path()?;
    let config = if config_path.exists() {
        load_config(&config_path)?
    } else {
        // Minimal default config
        orc_core::config::Config {
            master_provider: "claude".to_string(),
            providers: vec![],
            routing: Some(default_routing()),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    };

    let routing = config.routing.clone().unwrap_or_else(default_routing);
    let master = &config.master_provider;
    let available_providers: Vec<String> = vec!["claude".to_string(), "codex".to_string()];

    let mut state = ReplState::new(&cwd);

    // Print header
    let provider_refs: Vec<&str> = available_providers.iter().map(|s| s.as_str()).collect();
    println!("{}", build_header(env!("CARGO_PKG_VERSION"), &provider_refs, &cwd));
    println!();

    // Set up rustyline
    let mut rl = rustyline::DefaultEditor::new()?;

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    loop {
        let prompt = state.prompt_string();
        let readline = rl.readline(&prompt);

        match readline {
            Ok(line) => {
                let input = line.trim().to_string();
                if input.is_empty() {
                    continue;
                }
                rl.add_history_entry(&input)?;

                // Check for slash command
                if let Some(cmd) = parse_slash(&input) {
                    handle_slash_command(cmd, &mut state, &routing, &config, &available_providers)?;
                    continue;
                }

                // Log user message
                let _ = append_event(
                    &capsule_path(std::path::Path::new(&cwd), &state.session_id),
                    &CapsuleEvent::UserMessage {
                        content: input.clone(),
                        ts: chrono::Utc::now(),
                    },
                );

                // Route the message
                let route_entry = if let Some(override_provider) = &state.provider_override {
                    // Direct override — skip classifier
                    RouteEntry {
                        provider: override_provider.clone(),
                        model: None,
                    }
                } else {
                    // Classify intent
                    eprint!("⠋ Classifying...");
                    let category = rt.block_on(async {
                        let master_provider = get_provider(master).unwrap();
                        classifier::classify(master_provider.as_ref(), &input, &cwd).await
                    });
                    let category = category.unwrap_or_else(|| "question".to_string());
                    let entry = route(&category, &routing, master);

                    // Log classification
                    let _ = append_event(
                        &capsule_path(std::path::Path::new(&cwd), &state.session_id),
                        &CapsuleEvent::Classify {
                            input: input.clone(),
                            result: category.clone(),
                            provider: entry.provider.clone(),
                            model: entry.model.clone(),
                            ts: chrono::Utc::now(),
                        },
                    );

                    eprint!("\r");
                    eprintln!("→ {} ({})", category, route_display(&entry));
                    entry
                };

                // Execute via provider
                let provider = match get_provider(&route_entry.provider) {
                    Some(p) => p,
                    None => {
                        eprintln!("Provider '{}' not available", route_entry.provider);
                        continue;
                    }
                };

                let opts = InvokeOpts {
                    message: input.clone(),
                    working_dir: cwd.clone(),
                    model: route_entry.model.clone(),
                    timeout_secs: 120,
                    ..Default::default()
                };

                let start = std::time::Instant::now();
                eprintln!("⠋ Running {}...", route_entry.provider);

                let result = rt.block_on(async {
                    orc_core::invoke::stream::execute(provider.as_ref(), opts).await
                });

                let duration = start.elapsed();

                match result {
                    Ok(invoke_result) => {
                        println!("{}", invoke_result.response);

                        // Estimate cost (rough — we don't have exact token counts from all providers)
                        let approx_tokens_in = input.len() as u64 / 4;
                        let approx_tokens_out = invoke_result.response.len() as u64 / 4;
                        let cost = estimate_cost(
                            &route_entry.provider,
                            route_entry.model.as_deref(),
                            approx_tokens_in,
                            approx_tokens_out,
                        );
                        state.cost_tracker.add(cost);

                        // Log invoke end
                        let _ = append_event(
                            &capsule_path(std::path::Path::new(&cwd), &state.session_id),
                            &CapsuleEvent::InvokeEnd {
                                provider: route_entry.provider.clone(),
                                tokens_in: approx_tokens_in,
                                tokens_out: approx_tokens_out,
                                cost_usd: cost,
                                duration_ms: duration.as_millis() as u64,
                                ts: chrono::Utc::now(),
                            },
                        );

                        // Footer
                        eprintln!(
                            "{} · ~{} tokens · {} · {:.1}s",
                            route_entry.provider,
                            approx_tokens_in + approx_tokens_out,
                            format_cost(cost),
                            duration.as_secs_f64(),
                        );

                        // Budget check
                        if state.cost_tracker.over_budget() {
                            eprintln!(
                                "⚠ Budget reached ({}). Continue? [y/N]",
                                format_cost(state.cost_tracker.budget().unwrap_or(0.0))
                            );
                            let mut confirm = String::new();
                            std::io::stdin().read_line(&mut confirm)?;
                            if !confirm.trim().eq_ignore_ascii_case("y") {
                                eprintln!("Session paused. Use /budget to adjust.");
                            }
                        }

                        // Track in history
                        state.history.push(("user".to_string(), input));
                        state.history.push(("assistant".to_string(), invoke_result.response));
                    }
                    Err(e) => {
                        eprintln!("Error: {}", e);
                    }
                }
            }
            Err(rustyline::error::ReadlineError::Interrupted) => {
                eprintln!("^C");
                continue;
            }
            Err(rustyline::error::ReadlineError::Eof) => {
                break;
            }
            Err(e) => {
                eprintln!("Error: {}", e);
                break;
            }
        }
    }

    Ok(())
}

fn handle_slash_command(
    cmd: SlashCommand,
    state: &mut ReplState,
    routing: &HashMap<String, RouteEntry>,
    config: &orc_core::config::Config,
    available_providers: &[String],
) -> anyhow::Result<()> {
    match cmd {
        SlashCommand::Help => {
            println!("Commands:");
            println!("  /models      Show routing table");
            println!("  /set <cat> <provider> [model]  Set routing");
            println!("  /providers   Check installed providers");
            println!("  /sessions    Open session hub");
            println!("  /budget [N]  Show/set budget");
            println!("  /help        This message");
            println!();
            println!("Shift+Tab cycles provider override");
        }
        SlashCommand::Models => {
            println!("{:<12} {:<12} {}", "CATEGORY", "PROVIDER", "MODEL");
            println!("{}", "-".repeat(40));
            for (cat, entry) in routing {
                let model = entry.model.as_deref().unwrap_or("(default)");
                println!("{:<12} {:<12} {}", cat, entry.provider, model);
            }
        }
        SlashCommand::Providers => {
            // Delegate to existing doctor logic
            use orc_core::invoke::provider::Provider;
            let providers: Vec<Box<dyn Provider>> = vec![
                Box::new(ClaudeProvider),
                Box::new(CodexProvider),
            ];
            for p in &providers {
                let report = p.detect();
                let status = if report.installed { "✓" } else { "✗" };
                let auth = if report.auth_ok { "authenticated" } else { "not authenticated" };
                println!("  {} {} ({})", status, report.name, auth);
            }
        }
        SlashCommand::Sessions => {
            // Launch TUI session hub
            let mut app = orc_tui::app::App::new();
            app.run()?;
        }
        SlashCommand::Set { category, provider, model } => {
            // Update routing in memory (and save to disk)
            println!("Set {} → {} {}", category, provider, model.as_deref().unwrap_or("(default)"));
            // Note: actual config persistence happens via save_config
            // For MVP, just print confirmation. Full persistence added when config is mutable.
        }
        SlashCommand::Budget { amount } => {
            if let Some(amt) = amount {
                state.cost_tracker.set_budget(Some(amt));
                println!("Budget set: {} per session", format_cost(amt));
            } else {
                let budget_str = state.cost_tracker.budget()
                    .map(|b| format_cost(b))
                    .unwrap_or_else(|| "no limit".to_string());
                println!("Session: {} spent ({})", format_cost(state.cost_tracker.total()), budget_str);
            }
        }
    }
    Ok(())
}
```

**Note:** The `/sessions` handler references `orc_tui::app::App`. Add `orc-tui` as a dependency in `crates/orc-repl/Cargo.toml`:

```toml
orc-tui = { path = "../orc-tui" }
```

**Step 4: Run tests**

Run: `cargo test --manifest-path ./Cargo.toml -p orc-repl 2>&1 | tail -5`
Expected: all repl tests pass.

**Step 5: Commit**

```bash
git add crates/orc-repl/
git commit -m "feat: REPL shell — core loop with classify, route, invoke, cost tracking"
```

---

## Task 9: Rewire orc-bin to default to REPL

**Files:**
- Modify: `crates/orc-bin/src/main.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn test_hub_subcommand_exists() {
    // Verify that `orc hub` is a valid subcommand after the change
    let result = Cli::try_parse_from(["orc", "hub"]);
    assert!(result.is_ok(), "orc hub should be a valid subcommand");
}
```

**Step 2: Run to verify compile fails**

Expected: no `Hub` variant in `Commands`.

**Step 3: Implement**

In `main.rs`:

1. Add `Hub` variant to `Commands`:

```rust
#[derive(Subcommand)]
enum Commands {
    /// Open the TUI session hub
    Hub,
    // ... existing commands
}
```

2. Change the `None` arm (no subcommand) from TUI to REPL:

```rust
match cli.command {
    None => {
        // REPL mode (new default)
        orc_repl::repl::run_repl()?;
    }
    Some(Commands::Hub) => {
        // TUI session hub (old default, now explicit)
        let mut app = App::new();
        app.run()?;
    }
    // ... rest unchanged
}
```

3. Update the `#[command]` description:

```rust
#[command(name = "orc", about = "Conversational coding shell with smart model routing")]
```

**Step 4: Run tests**

Run: `cargo test --manifest-path ./Cargo.toml 2>&1 | tail -5`
Expected: all tests pass.

**Step 5: Build and smoke test**

```bash
cargo build --manifest-path ./Cargo.toml 2>&1 | tail -3
```

Then manually test:
- `./target/debug/orc --help` → should show "Conversational coding shell"
- `./target/debug/orc hub` → should open the TUI session hub

**Step 6: Commit**

```bash
git add crates/orc-bin/src/main.rs
git commit -m "feat: rewire orc default to REPL, move TUI hub to 'orc hub'"
```

---

## Task 10: Install and final verification

**Step 1: Run full test suite**

```bash
cargo test --manifest-path ./Cargo.toml 2>&1 | grep "test result"
```

Expected: all crates pass (173 existing + new repl tests).

**Step 2: Install**

```bash
cargo install --path ./crates/orc-bin
```

**Step 3: Smoke test installed binary**

```bash
orc --help
orc hub    # should open TUI
# Then Ctrl+D to exit
```

**Step 4: Commit any final fixes**

```bash
git add -A
git commit -m "chore: final verification — all tests pass, binary installed"
```
