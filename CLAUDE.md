# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What orc is for

Read `docs/plans/2026-11-08-orc-positioning.md` before changing what the product does. In short: orc gets a coding task done correctly by routing it to the best available agent, verifying the result, and handing off to another agent on failure — with a record of every step. It is **not** a terminal/workspace manager (that is herdr's ground). Build order: **verify → hand off → route**.

The older Go implementation lives in `../orc-go` and is no longer developed. This repo is Rust only.

## Build and Test

```bash
cargo build                             # whole workspace
cargo test                              # all tests (~230, all unit tests)
cargo test -p orc-core                  # one crate
cargo test -p orc-repl classifier       # tests matching a name
cargo run -p orc --                     # run the binary (REPL)
cargo run -p orc -- list --tool claude  # run a subcommand
cargo install --path crates/orc-bin     # install to ~/.cargo/bin
```

This laptop is shared and has 8 GB RAM: check `uptime` first and pass `-j 2` to cargo builds.

## Workspace layout

| Crate | Role |
|---|---|
| `orc-bin` | `orc` binary. clap CLI in `src/main.rs`: no subcommand → REPL; `run` (verified task), `hub`, `list`, `resume`, `export`, `import`, `flow create`, `init`, `providers doctor` |
| `orc-core` | Library: config, flows, provider invocation, session parsing, verification, run log |
| `orc-repl` | Full-screen conversational REPL with request classification and routing |
| `orc-tui` | ratatui session hub (`orc hub`): session list, embedded PTY view, flow runner |

### orc-core modules

| Module | Role |
|---|---|
| `config.rs` | `~/.orc/config.json`: master provider, registered providers, `routing` table (`RouteEntry { provider, model }`, `default_routing()`) |
| `flow/` | `compile.rs` turns a natural-language prompt into staged flows (`types.rs`); flows saved to `<repo>/.orc/flows/<name>.json` |
| `capsule.rs` | Append-only run log at `<repo>/.orc/runs/<id>/events.jsonl`, used by the REPL and `orc run` (`stage_started` with `attempt`/`handoff_from`, `agent_finished`, `check_result`, `review_result`, `verdict`, `handoff`). Re-exported as `orc_repl::capsule` |
| `relay.rs` | Step 2 of the loop. `run_relay` loops `run_stage` across providers until a pass or `max_attempts`; `next_provider` (pure hand-off policy: limit/stuck → another non-limited provider, fail → another or the same), `build_handoff_prompt` (task + verdict, evidence, previous response, `diff_stat`). Pins one base commit for the whole run; `--model` only for the first provider |
| `verify/` | Step 1 of the loop. `stage.rs` (`run_stage`: agent → checks → review → verdict, plus git base/diff), `verdict.rs` (`Verdict` pass/fail/stuck/limit, `decide` order, exit codes), `check.rs` (`--check` shell commands), `review.rs` (read-only reviewer prompt + JSON parsing), `limit.rs` (rate-limit patterns, scanned only in stderr and error events) |
| `invoke/` | `provider.rs` builds provider commands (`InvokeOpts` has `timeout_secs`, `idle_timeout_secs`, `read_only`), `providers/` holds one headless runner per CLI (`claude`, `codex`, `opencode`, `pi`, `kimi`, `amp`, `gemini`, `copilot`; registered in `providers/mod.rs` `get_provider`/`all_providers`). Each file's doc comment records the exact command and the output format it was verified against (live run, or the CLI's own source when the account couldn't run). `supports_read_only()` gates who may review (claude, codex, opencode, pi). Text-only CLIs use `gemini::plain_text_line`; amp reuses the Claude stream-json parser; `stream.rs` runs a provider with total/idle timeouts, stderr capture and process-group kill (`ExitReason`), `resume.rs` does native and **cross-tool resume** (`cross_tool_resume`, `build_cross_tool_command`) |
| `session/` | Parsers for each tool's on-disk session history (`amp`, `claude`, `codex`, `copilot`, `droid`, `gemini`, `kimi`, `opencode`), `index.rs` (unified index cached at `~/.orc/sessions.jsonl`), `handoff.rs` (`generate_handoff_markdown`), `scp.rs` (portable session context JSON, default `<repo>/.orc/context.json`), `types.rs` (`SessionContext`, `SessionSource`) |

### orc-repl modules

| Module | Role |
|---|---|
| `repl.rs` | Event loop; runs the active provider in an embedded PTY. Esc toggles `orc>` command mode; Shift+Tab switches to the next provider and hands it a brief of the current screen (`extract_screen_brief`) |
| `classifier.rs` | Builds a prompt asking the master provider to classify a request as `plan`, `implement`, `review`, `debug` or `question`; parses the JSON answer |
| `router.rs` | `route()` maps a category to a `RouteEntry` from config; unknown categories fall back to the master provider |
| `cost.rs` | Per-provider cost estimates and a running `CostTracker` |
| `slash.rs` | `/help`, `/models`, `/providers`, `/sessions`, `/clear`, `/set <category> <provider> [model]`, `/budget [amount]`, `/switch <provider>` |

## Files orc reads and writes

```
~/.orc/config.json                      # global config incl. routing table
~/.orc/sessions.jsonl                   # unified session index cache
<repo>/.orc/flows/<name>.json           # flow definitions
<repo>/.orc/runs/<id>/events.jsonl      # run log (REPL sessions and orc run)
<repo>/.orc/context.json                # exported session context (SCP)
```

`.orc/runs/`, `.orc/context.json` and `.orc/sessions.jsonl` are gitignored.

## Conventions

- Tests sit in `#[cfg(test)] mod tests` at the bottom of each file. Add tests for new parsers and command builders; they are the main safety net, since provider CLIs change output formats.
- Each provider gets its own file under `invoke/providers/` and `session/`. Don't add provider-specific branches elsewhere.
- Conventional Commits (`feat(repl): …`, `fix(core): …`).
