# orc Rust Rewrite — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build a standalone Rust TUI that acts as a session hub + multi-agent orchestrator across all coding CLI tools.

**Architecture:** 3-crate Rust workspace (orc-core for types/parsers/engine, orc-tui for ratatui UI, orc-bin for CLI entry). Session-hub-first — the default screen is a filterable session list. Flow orchestration and embedded PTY are secondary views.

**Tech Stack:** Rust, ratatui + crossterm (TUI), tokio (async), serde (parsing), rusqlite (OpenCode), clap (CLI), portable-pty (embed mode)

**Design doc:** `docs/plans/2026-02-20-orc-rust-rewrite-design.md`

---

## Milestone 1: Workspace Skeleton + Empty TUI

### Task 1: Initialize Rust workspace

**Files:**
- Create: `Cargo.toml` (workspace root)
- Create: `crates/orc-core/Cargo.toml`
- Create: `crates/orc-core/src/lib.rs`
- Create: `crates/orc-tui/Cargo.toml`
- Create: `crates/orc-tui/src/lib.rs`
- Create: `crates/orc-bin/Cargo.toml`
- Create: `crates/orc-bin/src/main.rs`

**Step 1: Create workspace root Cargo.toml**

```toml
[workspace]
members = ["crates/orc-core", "crates/orc-tui", "crates/orc-bin"]
resolver = "2"

[workspace.package]
version = "0.1.0"
edition = "2021"
license = "MIT"

[workspace.dependencies]
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
chrono = { version = "0.4", features = ["serde"] }
anyhow = "1"
```

**Step 2: Create orc-core crate**

`crates/orc-core/Cargo.toml`:
```toml
[package]
name = "orc-core"
version.workspace = true
edition.workspace = true

[dependencies]
tokio.workspace = true
serde.workspace = true
serde_json.workspace = true
chrono.workspace = true
anyhow.workspace = true
dirs = "6"
```

`crates/orc-core/src/lib.rs`:
```rust
pub mod config;
pub mod session;
```

**Step 3: Create orc-tui crate**

`crates/orc-tui/Cargo.toml`:
```toml
[package]
name = "orc-tui"
version.workspace = true
edition.workspace = true

[dependencies]
orc-core = { path = "../orc-core" }
ratatui = "0.29"
crossterm = "0.28"
tokio.workspace = true
anyhow.workspace = true
```

`crates/orc-tui/src/lib.rs`:
```rust
pub mod app;
pub mod theme;
pub mod views;
pub mod widgets;
```

**Step 4: Create orc-bin crate**

`crates/orc-bin/Cargo.toml`:
```toml
[package]
name = "orc"
version.workspace = true
edition.workspace = true

[[bin]]
name = "orc"
path = "src/main.rs"

[dependencies]
orc-core = { path = "../orc-core" }
orc-tui = { path = "../orc-tui" }
clap = { version = "4", features = ["derive"] }
tokio.workspace = true
anyhow.workspace = true
```

`crates/orc-bin/src/main.rs`:
```rust
fn main() {
    println!("orc v0.1.0");
}
```

**Step 5: Verify build**

Run: `cargo build`
Expected: Compiles with no errors.

**Step 6: Commit**

```bash
git init
echo "target/" > .gitignore
git add -A
git commit -m "feat: initialize rust workspace with 3 crates"
```

---

### Task 2: Minimal TUI that launches alternate screen

**Files:**
- Create: `crates/orc-tui/src/app.rs`
- Create: `crates/orc-tui/src/theme.rs`
- Create: `crates/orc-tui/src/views/mod.rs`
- Create: `crates/orc-tui/src/widgets/mod.rs`
- Modify: `crates/orc-bin/src/main.rs`

**Step 1: Write app.rs — minimal ratatui event loop**

```rust
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen,
    disable_raw_mode, enable_raw_mode,
};
use crossterm::execute;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Terminal;
use std::io::{self, stdout};

pub struct App {
    pub should_quit: bool,
}

impl App {
    pub fn new() -> Self {
        Self { should_quit: false }
    }

    pub fn run(&mut self) -> Result<()> {
        enable_raw_mode()?;
        execute!(stdout(), EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout());
        let mut terminal = Terminal::new(backend)?;

        let result = self.event_loop(&mut terminal);

        disable_raw_mode()?;
        execute!(stdout(), LeaveAlternateScreen)?;

        result
    }

    fn event_loop(&mut self, terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
        loop {
            terminal.draw(|frame| self.render(frame))?;

            if event::poll(std::time::Duration::from_millis(50))? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press {
                        match key.code {
                            KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
                            _ => {}
                        }
                    }
                }
            }

            if self.should_quit {
                return Ok(());
            }
        }
    }

    fn render(&self, frame: &mut ratatui::Frame) {
        let area = frame.area();
        let chunks = Layout::vertical([
            Constraint::Min(1),
            Constraint::Length(1),
        ]).split(area);

        let title = Line::from(" orc — session hub ").bold().cyan();
        let block = Block::default().borders(Borders::ALL).title(title);
        let body = Paragraph::new("No sessions loaded yet. Press q to quit.")
            .block(block);
        frame.render_widget(body, chunks[0]);

        let footer = Paragraph::new(" q quit")
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(footer, chunks[1]);
    }
}
```

**Step 2: Wire main.rs to launch TUI**

```rust
use orc_tui::app::App;

fn main() -> anyhow::Result<()> {
    let mut app = App::new();
    app.run()
}
```

**Step 3: Create stub modules**

`crates/orc-tui/src/theme.rs`: `// Theme constants — populated in Task 7`
`crates/orc-tui/src/views/mod.rs`: `// TUI views — populated in Task 7`
`crates/orc-tui/src/widgets/mod.rs`: `// TUI widgets — populated in Task 7`

**Step 4: Verify it works**

Run: `cargo run`
Expected: Full-screen alternate terminal with "No sessions loaded yet." Press `q` to exit cleanly.

**Step 5: Commit**

```bash
git add -A
git commit -m "feat: minimal ratatui TUI with alternate screen"
```

---

## Milestone 2: Core Types + Config

### Task 3: Session types

**Files:**
- Create: `crates/orc-core/src/session/mod.rs`
- Create: `crates/orc-core/src/session/types.rs`
- Test: `crates/orc-core/src/session/types.rs` (inline #[cfg(test)])

**Step 1: Write test for SessionSource serialization**

In `types.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_source_roundtrip() {
        let source = SessionSource::Claude;
        let json = serde_json::to_string(&source).unwrap();
        assert_eq!(json, "\"claude\"");
        let back: SessionSource = serde_json::from_str(&json).unwrap();
        assert_eq!(back, SessionSource::Claude);
    }

    #[test]
    fn unified_session_serializes() {
        let session = UnifiedSession {
            id: "abc123".into(),
            source: SessionSource::Codex,
            cwd: "/tmp/test".into(),
            repo: None,
            summary: "Fix auth bug".into(),
            updated_at: chrono::Utc::now(),
            created_at: chrono::Utc::now(),
            message_count: 5,
            original_path: "/home/user/.codex/sessions/abc.jsonl".into(),
        };
        let json = serde_json::to_string(&session).unwrap();
        assert!(json.contains("abc123"));
        assert!(json.contains("codex"));
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p orc-core`
Expected: FAIL — types not defined yet.

**Step 3: Implement types**

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionSource {
    Claude, Codex, Gemini, Copilot, OpenCode, Kimi, Amp, Droid, Scp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoState {
    pub branch: Option<String>,
    pub commit: Option<String>,
    pub dirty: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnifiedSession {
    pub id: String,
    pub source: SessionSource,
    pub cwd: PathBuf,
    pub repo: Option<RepoState>,
    pub summary: String,
    pub updated_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub message_count: usize,
    pub original_path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User, Assistant, System,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationMessage {
    pub role: Role,
    pub content: String,
    pub model: Option<String>,
    pub timestamp: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolType {
    Shell, FileEdit, FileRead, Grep, Search, Mcp, Patch, Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolActivity {
    pub tool_type: ToolType,
    pub summary: String,
    pub path: Option<String>,
    pub is_write: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input: u64,
    pub output: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionContext {
    pub session: UnifiedSession,
    pub messages: Vec<ConversationMessage>,
    pub tool_activity: Vec<ToolActivity>,
    pub files_modified: Vec<String>,
    pub reasoning_notes: Vec<String>,
    pub pending_tasks: Vec<String>,
    pub token_usage: Option<TokenUsage>,
    pub markdown: String,
}
```

**Step 4: Run tests**

Run: `cargo test -p orc-core`
Expected: PASS.

**Step 5: Commit**

```bash
git add -A
git commit -m "feat: core session types with serde"
```

---

### Task 4: Config module

**Files:**
- Create: `crates/orc-core/src/config.rs`
- Test: inline `#[cfg(test)]`

**Step 1: Write failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn config_roundtrip() {
        let cfg = Config {
            master_provider: "claude".into(),
            providers: vec![ProviderConfig {
                name: "claude".into(),
                binary: "claude".into(),
                installed: true,
                auth_status: AuthStatus::LoggedIn,
            }],
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let mut file = NamedTempFile::new().unwrap();
        save_config(file.path(), &cfg).unwrap();
        let loaded = load_config(file.path()).unwrap();
        assert_eq!(loaded.master_provider, "claude");
        assert_eq!(loaded.providers.len(), 1);
    }
}
```

**Step 2: Run test — FAIL**

Run: `cargo test -p orc-core config`
Expected: FAIL.

**Step 3: Implement config.rs**

```rust
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthStatus {
    Unknown, LoggedIn, LoggedOut, Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub name: String,
    pub binary: String,
    pub installed: bool,
    pub auth_status: AuthStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub master_provider: String,
    pub providers: Vec<ProviderConfig>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub fn config_dir() -> Result<std::path::PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("no home directory"))?;
    Ok(home.join(".orc"))
}

pub fn default_config_path() -> Result<std::path::PathBuf> {
    Ok(config_dir()?.join("config.json"))
}

pub fn load_config(path: &Path) -> Result<Config> {
    let data = std::fs::read_to_string(path)?;
    let config: Config = serde_json::from_str(&data)?;
    Ok(config)
}

pub fn save_config(path: &Path, config: &Config) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let data = serde_json::to_string_pretty(config)?;
    std::fs::write(path, data)?;
    Ok(())
}
```

Add `tempfile = "3"` to orc-core `[dev-dependencies]`.

**Step 4: Run test — PASS**

Run: `cargo test -p orc-core config`
Expected: PASS.

**Step 5: Commit**

```bash
git add -A
git commit -m "feat: config load/save with serde"
```

---

## Milestone 3: First Two Session Parsers

### Task 5: Claude session parser

**Files:**
- Create: `crates/orc-core/src/session/claude.rs`
- Test: inline `#[cfg(test)]`
- Reference: `https://github.com/yigitkonur/cli-continues/blob/main/src/parsers/claude.ts`

**Step 1: Write failing test with fixture data**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_claude_fixture(dir: &Path) -> PathBuf {
        // Mimics ~/.claude/projects/-Users-me-project/
        let project_dir = dir.join(".claude").join("projects").join("-Users-me-project");
        fs::create_dir_all(&project_dir).unwrap();
        let session_file = project_dir.join("abc123.jsonl");
        let lines = vec![
            r#"{"type":"human","message":{"role":"user","content":"Fix the auth bug"}}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":"I'll fix that."}}"#,
            r#"{"type":"human","message":{"role":"user","content":"Now add tests"}}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":"Done, tests added."}}"#,
        ];
        fs::write(&session_file, lines.join("\n")).unwrap();
        dir.to_path_buf()
    }

    #[tokio::test]
    async fn discovers_claude_sessions() {
        let tmp = TempDir::new().unwrap();
        let root = write_claude_fixture(tmp.path());
        let sessions = discover_claude_sessions_in(&root.join(".claude").join("projects")).await.unwrap();
        assert!(!sessions.is_empty());
        assert_eq!(sessions[0].source, SessionSource::Claude);
        assert!(sessions[0].summary.contains("Fix the auth bug"));
    }
}
```

**Step 2: Run test — FAIL**

Run: `cargo test -p orc-core claude`
Expected: FAIL.

**Step 3: Implement Claude parser**

Parse JSONL line by line. Extract `type: "human"` for user messages, `type: "assistant"` for assistant. Summary = first user message truncated. Session ID from filename.

Reference `https://github.com/yigitkonur/cli-continues/blob/main/src/parsers/claude.ts` for the exact JSONL schema.

**Step 4: Run test — PASS**

Run: `cargo test -p orc-core claude`

**Step 5: Commit**

```bash
git add -A
git commit -m "feat: claude session parser"
```

---

### Task 6: Codex session parser

**Files:**
- Create: `crates/orc-core/src/session/codex.rs`
- Test: inline `#[cfg(test)]`
- Reference: `https://github.com/yigitkonur/cli-continues/blob/main/src/parsers/codex.ts`

Same TDD pattern as Task 5. Codex stores JSONL under `~/.codex/sessions/`. Events use `type: "item.completed"` with `item.type: "agent_message"`.

**Step 1: Write failing test with fixture**

**Step 2: Run test — FAIL**

**Step 3: Implement parser**

**Step 4: Run test — PASS**

**Step 5: Commit**

```bash
git add -A
git commit -m "feat: codex session parser"
```

---

### Task 6b: Session index builder

**Files:**
- Create: `crates/orc-core/src/session/index.rs`
- Test: inline `#[cfg(test)]`

**Step 1: Write failing test**

Test that `build_index()` calls both parsers and returns a merged, sorted list. Uses fixture directories for both Claude and Codex.

**Step 2: Implement index.rs**

```rust
pub struct SessionIndex {
    pub sessions: Vec<UnifiedSession>,
    pub built_at: DateTime<Utc>,
}

pub async fn build_index(claude_root: &Path, codex_root: &Path) -> Result<SessionIndex> {
    let (claude, codex) = tokio::join!(
        discover_claude_sessions(claude_root),
        discover_codex_sessions(codex_root),
    );
    let mut all = Vec::new();
    all.extend(claude.unwrap_or_default());
    all.extend(codex.unwrap_or_default());
    all.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(SessionIndex { sessions: all, built_at: Utc::now() })
}
```

Plus `save_index` / `load_index` for `~/.orc/sessions.jsonl` caching with TTL.

**Step 3: Run test — PASS**

**Step 4: Commit**

```bash
git add -A
git commit -m "feat: session index builder with caching"
```

---

## Milestone 4: Session Hub TUI

### Task 7: Session list widget

**Files:**
- Create: `crates/orc-tui/src/widgets/session_list.rs`
- Create: `crates/orc-tui/src/theme.rs`
- Modify: `crates/orc-tui/src/widgets/mod.rs`

**Step 1: Implement theme.rs**

Define source badge colors (reference codex's `color.rs` for light/dark-aware blending):

```rust
use ratatui::style::{Color, Style, Modifier};

pub fn source_color(source: &str) -> Color {
    match source {
        "claude" => Color::Rgb(204, 120, 50),   // orange
        "codex" => Color::Rgb(0, 164, 84),       // green
        "gemini" => Color::Rgb(66, 133, 244),    // blue
        "copilot" => Color::Rgb(140, 86, 245),   // purple
        "opencode" => Color::Rgb(220, 220, 100), // yellow
        "kimi" => Color::Rgb(0, 191, 255),       // cyan
        _ => Color::Gray,
    }
}

pub fn footer_style() -> Style {
    Style::default().fg(Color::DarkGray)
}

pub fn selected_style() -> Style {
    Style::default().add_modifier(Modifier::REVERSED)
}
```

**Step 2: Implement session_list.rs**

Stateful list widget. Renders each session as:
`[source]  time_ago  project_name  summary...  session_id`

Supports: scroll (up/down), filter by source, search text.

Reference codex `selection_list.rs` for the scroll + selection pattern.

**Step 3: Verify renders**

Run: `cargo run`
Expected: Session list with real data from `~/.claude/projects/` and `~/.codex/sessions/`.

**Step 4: Commit**

```bash
git add -A
git commit -m "feat: session list widget with source badges"
```

---

### Task 8: Footer widget + key handling

**Files:**
- Create: `crates/orc-tui/src/widgets/footer.rs`
- Modify: `crates/orc-tui/src/app.rs`

**Step 1: Implement footer.rs**

Single-line widget that renders context-sensitive key hints:
`↑↓ navigate  enter resume  tab filter  / search  r resume-in-other  x export  f flow  q quit`

Reference codex `footer.rs` for width-based collapse logic.

**Step 2: Add key handling to app.rs**

- `↑/↓` or `j/k` — move selection
- `enter` — resume (Task 9)
- `tab` — cycle filter (all → claude → codex → ...)
- `/` — enter search mode
- `q` or `Esc` — quit

**Step 3: Verify**

Run: `cargo run`
Expected: Navigate sessions, filter by tool, search text, quit with q.

**Step 4: Commit**

```bash
git add -A
git commit -m "feat: footer widget and session hub key handling"
```

---

## Milestone 5: Resume (Spawn-and-Exit)

### Task 9: Provider resume commands

**Files:**
- Create: `crates/orc-core/src/invoke/mod.rs`
- Create: `crates/orc-core/src/invoke/resume.rs`
- Test: inline `#[cfg(test)]`
- Reference: `https://github.com/yigitkonur/cli-continues/blob/main/src/utils/resume.ts`

**Step 1: Write test for command building**

```rust
#[test]
fn claude_resume_command() {
    let cmd = build_resume_command(SessionSource::Claude, "abc123", "/tmp/project");
    assert_eq!(cmd.program, "claude");
    assert!(cmd.args.contains(&"--resume".to_string()));
    assert!(cmd.args.contains(&"abc123".to_string()));
}
```

**Step 2: Implement resume.rs**

```rust
pub struct ResumeCommand {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
}

pub fn build_resume_command(source: SessionSource, session_id: &str, cwd: &str) -> ResumeCommand {
    match source {
        SessionSource::Claude => ResumeCommand {
            program: "claude".into(),
            args: vec!["--resume".into(), session_id.into()],
            cwd: cwd.into(),
        },
        SessionSource::Codex => ResumeCommand {
            program: "codex".into(),
            args: vec!["-c".into(), format!("experimental_resume={}", session_id)],
            cwd: cwd.into(),
        },
        // ... etc
    }
}

/// Replaces current process with the CLI tool (exec).
pub fn spawn_and_exit(cmd: ResumeCommand) -> Result<()> {
    // Leave alternate screen, restore terminal, then exec
}
```

**Step 3: Wire enter key in app.rs**

On `enter`: leave alternate screen → `exec()` the resume command.

**Step 4: Test manually**

Run: `cargo run`, select a Claude session, press enter.
Expected: orc exits, Claude launches with that session resumed.

**Step 5: Commit**

```bash
git add -A
git commit -m "feat: spawn-and-exit resume for claude and codex"
```

---

## Milestone 6: Cross-Tool Handoff + SCP

### Task 10: Handoff markdown generator

**Files:**
- Create: `crates/orc-core/src/session/handoff.rs`
- Test: inline `#[cfg(test)]`
- Reference: `https://github.com/yigitkonur/cli-continues/blob/main/src/utils/markdown.ts`

Generate the structured markdown handoff document from `SessionContext`. Includes: overview table, tool activity, key decisions, recent conversation, files modified, pending tasks.

TDD: test that given a `SessionContext`, the markdown output contains expected sections.

---

### Task 11: SCP read/write

**Files:**
- Create: `crates/orc-core/src/session/scp.rs`
- Test: inline `#[cfg(test)]`

Serialize/deserialize `SessionContext` to/from `.orc/context.json`. The schema from the design doc Section 2.

TDD: roundtrip test — write SCP file, read it back, verify fields match.

---

### Task 12: Cross-tool resume in TUI

**Files:**
- Modify: `crates/orc-tui/src/app.rs`
- Modify: `crates/orc-core/src/invoke/resume.rs`

On `r` key: show a popup picker of target tools (excluding source). On selection: extract context → generate markdown → write SCP file + handoff.md → spawn target CLI with context injected.

Reference `https://github.com/yigitkonur/cli-continues/blob/main/src/utils/resume.ts` → `crossToolResume()` for per-tool CLI syntax.

---

## Milestone 7: Flow Engine

### Task 13: Flow compiler

**Files:**
- Create: `crates/orc-core/src/flow/mod.rs`
- Create: `crates/orc-core/src/flow/types.rs`
- Create: `crates/orc-core/src/flow/compile.rs`
- Test: inline `#[cfg(test)]`
- Reference: `internal/flow/flow.go`

Port `flow.Compile()` from Go. Same keyword matching + proximity-based provider assignment. Same `FlowDefinition` / `Stage` JSON shape so existing `.orc/flows/*.json` files work unchanged.

TDD: test that `compile("claude brainstorm, codex implement, kimi cross-check, review with opus")` produces 4 stages with correct providers.

---

### Task 14: Provider invoke trait + streaming

**Files:**
- Create: `crates/orc-core/src/invoke/provider.rs` (trait)
- Create: `crates/orc-core/src/invoke/stream.rs` (StreamEvent)
- Create: `crates/orc-core/src/invoke/claude.rs`
- Create: `crates/orc-core/src/invoke/codex.rs`
- Test: inline `#[cfg(test)]`
- Reference: `internal/invoke/invoke.go`

Port `invoke.Execute()` / `invoke.ExecuteStream()` from Go. Each provider builds subprocess args and decodes JSONL → `StreamEvent`.

TDD: test command building per provider. Test JSONL decoding with fixture data.

---

## Milestone 8: Flow Runner TUI

### Task 15: Transcript widget

**Files:**
- Create: `crates/orc-tui/src/widgets/transcript.rs`
- Reference: codex `chatwidget.rs`, `history_cell.rs`

Scrollable viewport that renders `StreamEvent`s. Live text gets `▌` left-bar prefix. Completed sections rendered as committed blocks.

---

### Task 16: Spinner / shimmer widget

**Files:**
- Create: `crates/orc-tui/src/widgets/spinner.rs`
- Reference: codex `shimmer.rs`

Time-based shimmer effect on streaming text. Sweep animation using elapsed time modulo period.

---

### Task 17: Flow runner view

**Files:**
- Create: `crates/orc-tui/src/views/flow_runner.rs`
- Create: `crates/orc-tui/src/widgets/stage_header.rs`
- Modify: `crates/orc-tui/src/app.rs`

Layout: stage header (pipeline progress) → transcript (streaming output) → footer (stage status + elapsed).

Entered via `f` on a session in the hub, or `orc --flow`.

Wire `tokio::process::Command` with piped stdout/stderr. Chunks arrive as `StreamEvent` via channel → TUI renders live.

---

## Milestone 9: Embedded PTY Mode

### Task 18: PTY viewport widget

**Files:**
- Create: `crates/orc-tui/src/widgets/pty_viewport.rs`
- Create: `crates/orc-tui/src/views/embedded.rs`
- Modify: `crates/orc-bin/Cargo.toml` (add `portable-pty`)

Spawn CLI tool in a pseudo-terminal. Read raw bytes → render in viewport. Forward keyboard input to PTY. `Esc` returns to hub.

This is the most complex widget. Reference codex's use of `utils/pty` crate for patterns.

---

## Milestone 10: Remaining Parsers

### Task 19: Gemini parser

**Files:** `crates/orc-core/src/session/gemini.rs`
Reference: `https://github.com/yigitkonur/cli-continues/blob/main/src/parsers/gemini.ts`
JSON files under `~/.gemini/tmp/*/chats/`.

### Task 20: Copilot parser

**Files:** `crates/orc-core/src/session/copilot.rs`
Reference: `https://github.com/yigitkonur/cli-continues/blob/main/src/parsers/copilot.ts`
YAML workspace + JSONL events under `~/.copilot/session-state/`.
Add `serde_yaml` dependency.

### Task 21: OpenCode parser

**Files:** `crates/orc-core/src/session/opencode.rs`
Reference: `https://github.com/yigitkonur/cli-continues/blob/main/src/parsers/opencode.ts`
SQLite DB at `~/.local/share/opencode/opencode.db`. Add `rusqlite` dependency.

### Task 22: Kimi, Amp, Droid parsers

**Files:** `crates/orc-core/src/session/kimi.rs`, `amp.rs`, `droid.rs`
Reference: `https://github.com/yigitkonur/cli-continues/blob/main/src/parsers/droid.ts` for Droid.

---

## Milestone 11: CLI Headless Commands

### Task 23: clap CLI with subcommands

**Files:**
- Modify: `crates/orc-bin/src/main.rs`

```
orc              → TUI (session hub)
orc list         → headless session list
orc resume <id>  → headless resume
orc export <id>  → write SCP file
orc import <f>   → import SCP
orc flow create  → headless flow creation
orc init         → provider detection
orc doctor       → provider health check
```

Wire each to the appropriate orc-core function. TUI is the default when no subcommand given.

---

## Summary

| Milestone | Tasks | What you get |
|-----------|-------|-------------|
| 1. Skeleton | 1-2 | Empty TUI launches in alternate screen |
| 2. Core types | 3-4 | Session types + config |
| 3. First parsers | 5-6b | Claude + Codex sessions discovered |
| 4. Session hub | 7-8 | Browse real sessions in TUI |
| 5. Resume | 9 | Press enter → CLI tool launches |
| 6. Handoff + SCP | 10-12 | Cross-tool resume + portable format |
| 7. Flow engine | 13-14 | Multi-stage flow compilation + invoke |
| 8. Flow runner | 15-17 | Streaming transcript in TUI |
| 9. Embedded PTY | 18 | Full CLI embed in TUI |
| 10. More parsers | 19-22 | All 8 tools supported |
| 11. Headless CLI | 23 | Non-TUI commands |
