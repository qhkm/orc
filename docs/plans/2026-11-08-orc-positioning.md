# orc Positioning — What orc Is For

**Date:** 2026-11-08
**Status:** Guiding doc. Supersedes the scope sections of the 2026-02-20 plans; their task-level detail still applies where it fits this direction.

## The one-line benefit

> Give orc a coding task. It gets it done **correctly**, by whichever agents are **best and available**, with **checked results** and a **record** of what happened.

orc is not a terminal or a workspace. It is the layer that decides **who** does the work, checks **whether** it is actually done, and keeps the work **moving** when an agent fails.

## Why this focus (vs. herdr)

[herdr](https://herdr.dev/) (Apache-2.0) is a live multi-agent workspace: persistent terminals in a background server (`herdr-core`), agent-spawned helpers (`hc run --bg`, `hc send`), per-agent status via the OSC 7501 terminal protocol, and remote machines over SSH. It does that well and for free.

| Benefit to the user | herdr | orc |
|---|---|---|
| See and steer many agents at once | **Yes** | No — don't build this |
| Best agent for each job (routing) | No, user picks | **Focus** |
| Work you can trust (verification) | No | **Focus** |
| Switch tools without losing context (handoff) | No | **Focus** |
| Record of what happened (audit) | No | **Focus** |

Rule: anything that is "a better place to watch agents run" is herdr's ground. orc either integrates with herdr or skips it.

## The loop

The three focus benefits are one loop, not three features:

```
Task
  │
  ▼
Route (A) ── pick best available agent for this kind of task
  │
  ▼
Run ──────── agent works (subprocess today; optionally a herdr pane)
  │
  ▼
Verify (B) ─ run acceptance checks (tests, build, lint, second-model review)
  │
  ├── pass ──────────────────────────────► Done + record
  │
  └── fail / stuck / rate-limited
        │
        ▼
      Hand off (C) ─ next agent gets full context + failure reason
        │
        └──────────────► back to Run
```

Every step writes to the run record (the capsule event log), which feeds back into Route.

## Build order

The order is forced by dependencies: each step needs the signal the previous one produces.

### 1. Verify (B) — first

Without a pass/fail signal, orc cannot know when to hand off or which routing works.

- Acceptance criteria become **executable**: `--accept "cargo test"` runs, it is not just text in a brief. Plain-language criteria ("docs updated") go to a reviewer agent that must answer pass/fail with reasons as structured JSON.
- Each stage ends with a verdict: `pass`, `fail(reason)`, `stuck`, `limit`.
- Verdicts and check output are appended to the capsule.
- Note: the Go version (`orc-go`) had `--accept` on `orc handoff`; the Rust rewrite does not yet. Port the concept, then enforce it.

**Done when:** a stage that breaks tests is reported as `fail` with the test output, without the user checking.

### 2. Hand off (C) — second

Most of the machinery exists: session parsers for 9 tools (`orc-core/src/session/`), `generate_handoff_markdown`, and `cross_tool_resume` / `build_cross_tool_command` (`orc-core/src/invoke/resume.rs`).

- Trigger handoff **automatically** on `fail`, `stuck` (no output for N minutes) or `limit` (detect rate-limit / quota messages per provider).
- The handoff brief includes the failure reason and check output, so the next agent starts from the failure, not from scratch.
- Cap retries per run; after the cap, stop and ask the user.
- Agent-callable delegation (adopted from herdr's best idea): `orc delegate --to <provider> --goal ... --run <id>` so an agent can hand off a sub-task mid-work, with orc still routing, verifying and recording.

**Done when:** a Claude rate limit mid-task continues in Codex with full context and the run record shows both legs.

### 3. Route (A) — third

The REPL already has an LLM classifier (`orc-repl/src/classifier.rs`), a routing table (`router.rs`) and cost estimates (`cost.rs`). Today routing is configured by hand.

- Record per-run: task category, provider, verdict, retries, cost, duration.
- Route on **observed pass rate and cost per category**, with the configured table as the cold-start default.
- Show why a route was chosen (`codex: 82% pass on "implement", $0.40 avg`).

**Done when:** orc's routing choices change based on its own history, and the user can see why.

## What to keep, shrink, add

| Keep (core of the loop) | Shrink (herdr's ground) | Add |
|---|---|---|
| Session parsers + cross-tool resume | TUI session hub as a primary product — keep only as a run/history viewer | Verify step + verdicts |
| Handoff markdown generation | Any live-terminal / multiplexing ambitions (`pty_screen`, embedded views) | Auto-handoff on fail / stuck / limit |
| Flow compiler (`flow/compile.rs`) | | `orc delegate` for agent-initiated handoff |
| REPL + classifier + router + cost | | Pass-rate log feeding the router |
| Capsule event log | | OSC 7501 status output per stage |
| | | Optional herdr backend: run stages in herdr panes |

## herdr integration (optional, after step 2)

- **Emit OSC 7501** working / done / needs-you per stage. orc runs then show live status inside herdr for free, and orc's own TUI can read the same signal.
- **herdr backend:** run a stage in a herdr pane instead of a one-shot subprocess, gaining persistence, live viewing and SSH machines without building them.

## Non-goals

- A terminal multiplexer, desktop app, or workspace manager.
- Real-time chat between agents for its own sake. Agents communicate through verified handoffs and `orc delegate`, which keeps every exchange on the record.

## Housekeeping before starting

1. **Commit or stash the in-progress work on `main`.** As of 2026-11-08 the entire `crates/orc-repl/` crate and `crates/orc-tui/src/widgets/pty_screen.rs` are untracked, and 10 tracked files have uncommitted changes (~670 lines). None of the REPL work is in git history yet.
2. **Rewrite `CLAUDE.md` and `AGENTS.md`.** They still describe the Go layout (`cmd/orc`, `internal/*`, `go build`). The repo is Rust (`orc-core`, `orc-tui`, `orc-repl`, `orc-bin`; `cargo build` / `cargo test`).
3. **Update `README.md`** commands to match the Rust CLI (`hub`, `list`, `resume`, `export`, `import`, `flow`, `init`, `providers`) and this positioning.
