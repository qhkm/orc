# Step 2: Hand off — Plan

**Date:** 2026-11-08
**Parent:** `2026-11-08-orc-positioning.md` (build order: verify → **hand off** → route). Builds on step 1 (`orc run` verdicts).
**Status:** Implemented 2026-11-08. Live-verified: `orc run --provider codex --fallback claude --check ./test.sh` on a repo with a broken `add()` — Codex hit its real usage limit, orc handed off, Claude fixed it from the brief, check passed: `PASS after 2 attempts (codex → claude)`, exit 0; run log shows `stage_started(1, codex) → verdict(limit) → handoff(codex→claude) → stage_started(2, claude, handoff_from codex) → check_result → verdict(pass)`.

## Goal

When a stage does not pass, orc keeps the work moving: it starts another attempt, by another agent when one is available, carrying the failure and the work so far. The user only hears back when the task passes or orc runs out of attempts.

**Done when:** a rate-limited agent mid-task is followed automatically by another agent that finishes the task, and the run log shows both attempts and the handoff between them. (Codex is rate-limited on 2026-11-08, so the live test runs Codex → Claude.)

## CLI

```bash
orc run "<task>" --provider codex --fallback claude --max-attempts 3 --check "cargo test"
```

- `--fallback a,b`: providers to hand off to, in order. Default: every other installed provider that can run headless (today `claude`, `codex`). `--no-fallback` disables handoff to other providers.
- `--max-attempts N`: total attempts including the first (default 3). `1` = step 1 behaviour.
- Exit code is the final attempt's verdict.

## Who gets the next attempt

| Verdict | Next attempt |
|---|---|
| `pass` | none, done |
| `limit` | next provider in the list that has not hit a limit in this run; none left → stop |
| `stuck` | next provider that has not hit a limit, other than the stuck one; none → stop |
| `fail` | next provider that has not hit a limit, other than the failing one; if there is no other, the **same** provider again, with the failure as feedback |

Providers are tried round-robin from the current one. Attempts stop at `--max-attempts`.

## Handoff brief

The next agent gets the original task plus a handoff section:

- which attempt failed, by whom, and the verdict with its reason and evidence (check output, reviewer reasons, limit message);
- what the previous agent said it did (tail of its final response);
- what is already changed in the repo since the run started (`git diff --stat` against the run's base commit, plus new untracked files);
- the checks and criteria it must satisfy;
- an instruction to continue from the current state, not start over.

The repo is not reset between attempts: work carries over. All attempts diff against the **first** attempt's base commit, so the reviewer sees the whole run's changes even if an agent committed. The reviewer always sees the original task, not the handoff prompt.

## Run log

One `run_id` for the whole run. `stage_started` gains `attempt` and `handoff_from`; a new `handoff` event (`from`, `to`, `attempt`, `reason`) is written between attempts. Old logs still parse (new fields default).

## Code

| Where | Change |
|---|---|
| `orc-core/src/relay.rs` (new) | `next_provider` policy (pure), `build_handoff_prompt`, `run_relay` (loops `run_stage`), `RelayReport` |
| `orc-core/src/verify/stage.rs` | `StageSpec` gains `attempt`, `agent_prompt` (what the agent receives; `task` stays the original), `base_commit` (pin the run's base), `handoff_from`; `diff_stat` helper |
| `orc-core/src/capsule.rs` | `stage_started.attempt` / `handoff_from`; `handoff` event |
| `orc-core/src/testutil.rs` | `ShellProvider::named` so tests can have several providers |
| `orc-bin/src/main.rs` | `--fallback`, `--no-fallback`, `--max-attempts`; per-attempt output; JSON report lists attempts |

## Out of scope (later)

- `orc delegate`, for agents to hand off a sub-task mid-work (positioning doc, step 2b).
- Enriching the brief with the previous agent's full session transcript (`session/handoff.rs` already renders one from on-disk history).
- Waiting out a rate limit and retrying the same provider.
- Resetting the repo between attempts.
