# ORC REPL + Smart Routing Design

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Evolve orc from a TUI session hub into a conversational coding shell with LLM-powered model routing — one REPL, multiple providers, smart task classification.

**Architecture:** REPL shell (default entry point) → intent classifier (master provider) → router (category → provider + model) → streaming execution (existing invoke layer). TUI session hub moves behind `/sessions` slash command.

**Tech Stack:** Rust, rustyline, ratatui (for /sessions), orc-core (unchanged), Claude API (classifier), Claude CLI + Codex CLI (execution)

---

## Architecture

```
┌─────────────────────────────────┐
│  REPL Shell (rustyline/ratatui) │  ← new default entry point
├─────────────────────────────────┤
│  Intent Classifier              │  ← master provider classifies task type
├─────────────────────────────────┤
│  Router                         │  ← maps classified intent → provider + model
├─────────────────────────────────┤
│  Provider Execution             │  ← existing: subprocess + stream parse
├─────────────────────────────────┤
│  orc-core (unchanged)           │  ← parsers, SCP, flow, config, invoke
└─────────────────────────────────┘
```

### Crate layout

```
crates/
  orc-core/   # session parsers, SCP, flow, invoke (unchanged)
  orc-tui/    # ratatui TUI — session hub, widgets (unchanged, behind `orc hub`)
  orc-repl/   # NEW — REPL shell, classifier, router, slash commands
  orc-bin/    # clap CLI entry point — rewired to default to REPL
```

---

## REPL Shell

### Entry point

`orc` (no args) opens a full-screen TUI (Codex CLI style):

```
┌─ orc v0.1.0 · claude + codex · ~/myproject ─────┐
│                                                    │
│ → implement (codex)                                │
│                                                    │
│ Created src/middleware/rateLimit.ts                 │
│   - Token bucket implementation                    │
│                                                    │
│ codex · 1,240 tokens · $0.02 · 8.3s               │
│                                                    │
├────────────────────────────────────────────────────┤
│ > _                                                │
├────────────────────────────────────────────────────┤
│ auto-route   ⇧Tab override   $0.02   /help        │
└────────────────────────────────────────────────────┘
```

**Layout (top to bottom):**
- Header bar: version, available providers, cwd
- Output pane (scrollable): streaming responses, routing decisions, slash command results
- Input box: tui-textarea for typing tasks / slash commands
- Status bar: active provider/override, session cost, hint keys

**Built with:** ratatui + crossterm + tui-textarea (same stack as orc-tui session hub)

### Input handling

- Plain text + Enter → task intent, sent to classifier
- `/command` + Enter → slash command, handled locally (no LLM call)
- `Shift+Tab` → cycle active provider override (shown in status bar)
- `PageUp/PageDown` → scroll output pane
- `Ctrl+C` → cancel current execution (or exit if idle)
- `Ctrl+D` → exit

### Provider override (Shift+Tab)

Status bar changes:

```
auto-route   ⇧Tab override   $0.02   /help     ← default
codex        ⇧Tab override   $0.02   /help     ← after Shift+Tab
claude       ⇧Tab override   $0.02   /help     ← again
auto-route   ⇧Tab override   $0.02   /help     ← cycles back
```

When overridden, classifier is skipped — message goes directly to that provider.

### Session continuity

- Each REPL session gets a run ID
- Conversation history persists within the session (multi-turn)
- On exit, writes SCP to `.orc/context.json`

---

## Intent Classifier

### How it works

1. Send user message to master provider (Claude) with structured prompt:

```
Classify this coding task. Respond with JSON only.
Task: "{user_message}"
Categories: plan, implement, review, debug, question
```

2. Parse response → get category
3. Router looks up provider + model from config

### Routing table

| Category | Provider | Model | Reasoning |
|---|---|---|---|
| `plan` | claude | opus | Deep reasoning for architecture |
| `implement` | codex | default | Fast code generation |
| `review` | claude | sonnet | Good analysis, cheaper than opus |
| `debug` | claude | opus | Strong reasoning for root cause |
| `question` | claude | haiku | Fast, cheap for simple answers |

### Config

```json
{
  "routing": {
    "plan": { "provider": "claude", "model": "opus" },
    "implement": { "provider": "codex", "model": null },
    "review": { "provider": "claude", "model": "sonnet" },
    "debug": { "provider": "claude", "model": "opus" },
    "question": { "provider": "claude", "model": "haiku" }
  }
}
```

`model: null` means use provider's default.

### Cost & latency

- ~100 input tokens, ~10 output tokens per classification
- ~0.1s latency
- Skipped when provider override is active
- Fallback: if classification fails, default to master provider

---

## Slash Commands (MVP)

| Command | Action |
|---|---|
| `/sessions` | Opens TUI session hub inline (existing ratatui code) |
| `/models` | Shows current routing table |
| `/set <category> <provider> [model]` | Update routing for a category |
| `/providers` | Runs provider doctor check |
| `/budget [amount]` | Show spend / set session budget cap |
| `/help` | List available commands |

### `/sessions` behavior

Temporarily switches terminal into full-screen ratatui TUI (existing session hub). On session select or `q`, returns to REPL. If session is selected for resume, resumes from REPL context.

### `/set` examples

```
/set implement codex          # provider only, keep default model
/set plan claude opus         # provider + model
/set review claude sonnet     # provider + model
```

### `/budget` behavior

```
> /budget
Session: $0.12 spent (no limit)

> /budget 5.00
Budget set: $5.00 per session
```

When budget hit: `Budget reached ($5.00). Continue? [y/N]`

Cost from token counts × per-model pricing (hardcoded lookup table).

---

## Execution Flow

### Single task lifecycle

```
> add rate limiting middleware to the API

⠋ Classifying...
→ implement (codex)

⠋ Running codex...
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Created src/middleware/rateLimit.ts
Modified src/index.ts
Created tests/rateLimit.test.ts

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
codex · 1,240 tokens · $0.02 · 8.3s

>
```

Steps:
1. User types message
2. Classifier call → `{"category": "implement"}`
3. Router: implement → codex (default model)
4. Show routing decision: `→ implement (codex)`
5. Build invoke command, stream output
6. Render inline with file write summaries
7. Show footer: provider, tokens, cost, duration
8. Append to capsule log
9. Return to prompt

### Multi-turn context

Each message includes prior conversation (capped at last 10 turns). When provider switches between turns, handoff markdown (existing SCP) carries context.

### Capsule logging

Append-only events to `.orc/runs/<session_id>/events.jsonl`:

```jsonl
{"type":"classify","input":"add rate limiting","result":"implement","provider":"codex","model":null,"ts":"..."}
{"type":"invoke_start","provider":"codex","tokens_in":340,"ts":"..."}
{"type":"invoke_end","provider":"codex","tokens_out":900,"cost":0.02,"duration_ms":8300,"ts":"..."}
```

---

## What's Deferred (Post-MVP)

| Feature | Why |
|---|---|
| Multi-stage pipeline (plan → implement → review) | MVP is smart single-agent. Pipeline is Layer 4. |
| Conversational workflow editing | Requires pipeline to exist. |
| Harness presets (balanced/fast/paranoid) | `/set` covers same ground manually. |
| Patch normalizer | Agents write files. Git handles diffs. |
| Parallel model arbitration | Cool but complex. |
| Role graph | Sequential stages fine for now. |
| `/dry-run`, `/details` | Need pipeline to simulate/view. |
| Full capsule | events.jsonl covers 80% of audit need. |

### Path from MVP to pipeline

1. `/plan` command → classifier proposes multi-stage plan
2. Stage executor → runs stages sequentially via SCP
3. Approval gates → "Proceed? [Y/n]" between stages
4. `/details` → show/collapse stage list

Each step independently shippable.

---

## MVP Scope Summary

- REPL shell as default `orc` entry point
- Intent classifier via master provider (Claude)
- Router: category → provider + model from config
- Shift+Tab provider override
- 6 slash commands (`/sessions`, `/models`, `/set`, `/providers`, `/budget`, `/help`)
- Streaming execution with inline output rendering
- Capsule logging (events.jsonl)
- Cost tracking with optional budget cap
- `orc hub` for explicit TUI session hub access
- MVP providers: Claude + Codex
