# orc Rust Rewrite — Design Document

**Date:** 2026-02-20
**Status:** Approved
**Approach:** Standalone Rust workspace (Approach A)

## Overview

New standalone Rust binary that combines:
- **Session hub** (primary) — discover, browse, resume, and cross-tool handoff sessions across all coding CLIs
- **Multi-agent orchestration** — define staged flows that pipe between providers
- **Full-screen TUI** — ratatui + crossterm, codex CLI-style alternate screen
- **Shared Context Protocol (SCP)** — portable session format any tool can adopt

Existing projects (`orc` Go, `cli-continues` TypeScript) stay alive separately.

## Crate Structure

```
orc/
├── Cargo.toml                 # workspace root
├── crates/
│   ├── orc-core/              # Types, session parsers, flow engine, provider invoke
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── session/       # Session discovery, parsing, index, handoff
│   │       │   ├── mod.rs
│   │       │   ├── types.rs   # UnifiedSession, SessionSource, SessionContext
│   │       │   ├── index.rs   # Build/cache unified session index
│   │       │   ├── claude.rs  # Claude JSONL parser
│   │       │   ├── codex.rs   # Codex JSONL parser
│   │       │   ├── gemini.rs  # Gemini JSON parser
│   │       │   ├── copilot.rs # Copilot YAML+JSONL parser
│   │       │   ├── opencode.rs# OpenCode SQLite parser
│   │       │   ├── kimi.rs    # Kimi parser
│   │       │   ├── amp.rs     # Amp parser
│   │       │   ├── droid.rs   # Factory Droid parser
│   │       │   ├── scp.rs     # Shared Context Protocol read/write
│   │       │   └── handoff.rs # Markdown handoff generation
│   │       ├── flow/          # Flow definitions, compile, stage pipeline
│   │       ├── invoke/        # Provider trait, subprocess builder, JSONL decoders
│   │       └── config.rs      # Global config (~/.orc/config.json)
│   │
│   ├── orc-tui/               # Full-screen ratatui application
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── app.rs         # Main app state machine + event loop
│   │       ├── views/         # Session hub, flow runner, provider embed
│   │       │   ├── mod.rs
│   │       │   ├── session_hub.rs
│   │       │   ├── flow_runner.rs
│   │       │   └── embedded.rs
│   │       ├── widgets/       # Reusable TUI widgets
│   │       │   ├── mod.rs
│   │       │   ├── session_list.rs
│   │       │   ├── transcript.rs
│   │       │   ├── composer.rs
│   │       │   ├── footer.rs
│   │       │   ├── spinner.rs
│   │       │   ├── stage_header.rs
│   │       │   └── pty_viewport.rs
│   │       └── theme.rs
│   │
│   └── orc-bin/               # CLI binary entry point
│       └── src/
│           └── main.rs        # clap args -> TUI or headless dispatch
```

### Key Dependencies

- `ratatui` + `crossterm` — TUI rendering
- `tokio` — async subprocess, streaming
- `serde` + `serde_json` — parsing
- `rusqlite` — OpenCode sessions
- `clap` — CLI args
- `dirs` — home directory resolution
- `portable-pty` — embedded provider mode
- `chrono` — timestamps

## Session Management

### Layer 1 — Native Parsers (read-only)

| Tool | Format | Location |
|------|--------|----------|
| Claude Code | JSONL | `~/.claude/projects/` |
| Codex | JSONL | `~/.codex/sessions/` |
| Gemini | JSON | `~/.gemini/tmp/*/chats/` |
| Copilot | YAML+JSONL | `~/.copilot/session-state/` |
| OpenCode | SQLite | `~/.local/share/opencode/` |
| Kimi | TBD | `~/.kimi/` |
| Amp | TBD | TBD |
| Droid | JSONL+JSON | `~/.factory/sessions/` |

All parsers produce `UnifiedSession`. Full context extraction produces `SessionContext`.

Session index cached at `~/.orc/sessions.jsonl` with 5-minute TTL. Built by calling all parsers in parallel via `tokio::join!`.

### Layer 2 — Shared Context Protocol (SCP)

Portable JSON format at `<project>/.orc/context.json` that any CLI tool can read/write:

```json
{
  "version": "1",
  "session_id": "abc123",
  "source": "claude",
  "created_at": "2026-02-20T10:00:00Z",
  "updated_at": "2026-02-20T10:30:00Z",
  "cwd": "/Users/me/project",
  "repo": {
    "branch": "main",
    "commit": "a1b2c3d",
    "dirty": true
  },
  "messages": [
    { "role": "user", "content": "Add auth middleware", "timestamp": "..." },
    { "role": "assistant", "content": "I'll create...", "model": "claude-sonnet-4-5", "timestamp": "..." }
  ],
  "tool_activity": [
    { "type": "shell", "command": "npm test", "exit_code": 0, "summary": "npm test -> exit 0" },
    { "type": "file_edit", "path": "src/auth.ts", "action": "edit", "summary": "edit src/auth.ts (+45 -3)" }
  ],
  "files_modified": ["src/auth.ts", "src/middleware.ts"],
  "reasoning_notes": ["Need to handle token refresh racing with logout"],
  "pending_tasks": ["Add tests for auth middleware"],
  "token_usage": { "input": 45230, "output": 12847 }
}
```

- `orc export <id>` writes SCP file
- `orc import context.json --in codex` launches target with context
- Auto-detected when `.orc/context.json` exists in cwd

## TUI Layout

### View 1 — Session Hub (default)

Filterable session list with tool badges, timestamps, summaries. Key bindings:
- `enter` — spawn-and-exit (native resume in same tool)
- `r` — cross-tool handoff (pick target, spawn-and-exit)
- `tab` — filter by tool/directory
- `/` — search
- `f` — run flow on selected session's cwd
- `e` — embed provider in TUI
- `x` — export as SCP

### View 2 — Flow Runner

Shows stage pipeline progress at top, scrollable streaming transcript in middle, footer with stage status + elapsed time. Stages auto-advance. Follow-up prompt after completion (review/cross-check/refine).

Streaming rendered with `▌` left-bar prefix on live text. Shimmer effect on tokens (reference codex's `shimmer.rs`).

### View 3 — Embedded Provider

PTY-based terminal embed via `portable-pty`. Raw terminal output in viewport. Full interactivity with underlying CLI. `esc` returns to hub.

### Widget inventory

| Widget | File | Role | Codex reference |
|--------|------|------|-----------------|
| Session list | `session_list.rs` | Filterable scrollable list | `selection_list.rs` |
| Transcript | `transcript.rs` | Scrollable output viewport | `chatwidget.rs` |
| Composer | `composer.rs` | Text input for flow prompts | `chat_composer.rs` |
| Footer | `footer.rs` | Key hints + status | `footer.rs` |
| Spinner | `spinner.rs` | Shimmer/progress | `shimmer.rs` |
| Stage header | `stage_header.rs` | Pipeline progress | (new) |
| PTY viewport | `pty_viewport.rs` | Embedded terminal | (new) |

## Provider Invocation

### Provider trait

```rust
pub trait Provider: Send + Sync {
    fn name(&self) -> &str;
    fn binary(&self) -> &str;
    fn build_command(&self, opts: &InvokeOpts) -> Command;
    fn parse_stream(&self, line: &str) -> Option<StreamEvent>;
    fn detect(&self) -> DoctorReport;
}
```

One implementation per provider file. Adding a new provider = one file.

### StreamEvent (common output)

```rust
pub enum StreamEvent {
    Text(String),
    ToolStart { name: String, input: String },
    ToolEnd { name: String, output: String },
    Reasoning(String),
    Status(String),
    Error(String),
    Done,
}
```

### Execution modes

| Mode | Trigger | Mechanism |
|------|---------|-----------|
| Spawn-and-exit | `enter` on session, `orc resume` | `exec()` replaces process |
| Streamed stage | Flow runner | `tokio::process::Command` piped, chunks as `StreamEvent` |
| Embedded PTY | `e` on session | `portable-pty`, raw bytes in viewport |

### Scope guard

Fast regex heuristic (`is_likely_coding_request`) first. Fallback: master provider classifies via structured JSON prompt. Runs before each flow turn.

## Data Model

### Core types

- `SessionSource` — enum of 8+ tools
- `UnifiedSession` — id, source, cwd, repo, summary, timestamps, message_count
- `SessionContext` — full extraction: messages, tool_activity, files, reasoning, markdown
- `ConversationMessage` — role, content, model, timestamp
- `ToolActivity` — tool_type, summary, path, is_write
- `SessionIndex` — cached list of UnifiedSession, built_at, TTL

### Config (`~/.orc/config.json`)

- `master_provider`, `providers[]` (name, binary, installed, auth_status), timestamps
- Same shape as Go version

### Flow definitions (`<project>/.orc/flows/*.json`)

- `FlowDefinition` with name, stages[], master_provider
- Same JSON shape as Go version — existing flow files work unchanged

## Binary Modes

- `orc` — full-screen TUI (session hub)
- `orc list` — headless session list
- `orc resume <id> [--in <tool>]` — headless resume/handoff
- `orc export <id>` — write SCP file
- `orc import <file> --in <tool>` — import SCP and launch tool
- `orc flow create ...` — headless flow creation
- `orc init` — provider detection + config
- `orc providers doctor` — health check
