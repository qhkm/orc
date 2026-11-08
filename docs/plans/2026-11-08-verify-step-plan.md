# Step 1: Verify — Plan

**Date:** 2026-11-08
**Parent:** `2026-11-08-orc-positioning.md` (build order: verify → hand off → route)
**Status:** Implemented 2026-11-08. Live-verified against real CLIs: Claude (haiku) breaking `add()` → `FAIL` with `expected 5, got -1`, exit 1; Claude writing a README judged by a read-only Claude reviewer → `PASS`, exit 0; rate-limited Codex → `LIMIT` with the real quota message, exit 3. `STUCK` covered by unit tests that kill a silent process.

## Goal

A stage ends with a verdict orc decided, not the agent: `pass`, `fail`, `stuck` or `limit`, with evidence, recorded in the run log.

**Done when:** `orc run` on a throwaway repo with a failing check reports `fail` with the check output as evidence, with no manual checking.

## Findings that shaped the plan

- No headless stage execution existed in Rust. The TUI flow runner compiled flows but never invoked a stage; the REPL runs agents interactively in a PTY with no "done" signal. Only the classifier called `invoke::execute`.
- `InvokeOpts.timeout_secs` was never enforced, and stderr was piped but never read (rate-limit messages go there; a full pipe can also block the child).
- Both headless runners were broken against current CLIs:
  - `codex -q` is rejected by codex-cli 0.166 (`unexpected argument '-q'`). Current form: `codex exec --json`.
  - `claude -p --output-format stream-json` now fails with `requires --verbose` (Claude Code 2.1.319). This also silently broke the REPL classifier.
- Real Codex usage-limit output captured on 2026-11-08 (used as a test fixture):
  `{"type":"error","message":"You've hit your usage limit. ..."}` followed by `{"type":"turn.failed",...}`.

## CLI

```bash
orc run "<task>" [--provider <p>] [--model <m>] \
  [--check "<shell cmd>"]... [--accept "<plain-language criterion>"]... \
  [--reviewer <p>] [--timeout <secs>=1800] [--idle <secs>=300] [--json]
```

- `--check`: shell command run in the repo after the agent finishes. Exit 0 = pass. Output tail kept as evidence.
- `--accept`: judged by a reviewer agent from the diff since the run started (base commit recorded before the run, so agent commits are included) plus untracked files. Reviewer answers JSON with pass/fail and a reason per criterion; a criterion it doesn't answer counts as fail.
- `--reviewer` defaults to the master provider; orc warns if it is the same as the implementer.
- Exit codes: `0` pass, `1` fail, `2` stuck, `3` limit, `4` orc error.

## Verdict order

First match wins:

1. `stuck` — agent produced no output for `--idle` seconds or ran past `--timeout`; process killed. Checks skipped.
2. `limit` — rate-limit / quota message in stderr or provider error events. Checks skipped.
3. `fail` — agent exited non-zero or reported an error.
4. `fail` — any `--check` failed (all checks run, so evidence is complete).
5. `fail` — reviewer rejected a criterion, or its answer could not be parsed.
6. `pass`.

Limit detection only scans stderr and provider error events, never the agent's normal text, so an agent writing a rate limiter does not trigger it.

## Code

| Where | Change |
|---|---|
| `orc-core/src/invoke/stream.rs` | Read stderr concurrently; enforce overall and idle timeouts (kill on expiry); `InvokeResult` gains `stderr`, `errors`, `exit_reason` |
| `orc-core/src/invoke/provider.rs` | `InvokeOpts.idle_timeout_secs` |
| `orc-core/src/invoke/providers/claude.rs` | Add `--verbose`; `result` with `is_error` → `Error` event |
| `orc-core/src/invoke/providers/codex.rs` | `codex exec --json --skip-git-repo-check -s workspace-write -C <dir> [-m]`; parse `item.completed`/`agent_message`, `error`, `turn.failed`, `turn.completed` |
| `orc-core/src/capsule.rs` | Moved from `orc-repl` (re-exported there); new events `stage_started`, `check_result`, `review_result`, `verdict` |
| `orc-core/src/verify/` | `verdict.rs` (types, `decide`, exit codes), `check.rs`, `limit.rs`, `review.rs`, `stage.rs` (`run_stage`, git base/diff) |
| `orc-bin/src/main.rs` | `orc run` |

## Tests

Unit tests with a test-only shell provider (`sh -c <script>`) so no real agent is needed: timeouts, stderr capture, limit patterns (real Codex fixture), reviewer JSON parsing, verdict order, and `run_stage` end to end (agent edits a file → check fails → `fail` with evidence; reviewer rejects → `fail`; all good → `pass`).

## Out of scope

Auto-handoff (step 2), verification inside the interactive REPL, the TUI flow runner, multi-stage `--flow` runs.
