# Step 3: Route — Plan

**Date:** 2026-11-08
**Parent:** `2026-11-08-orc-positioning.md` (build order: verify → hand off → **route**). Builds on step 1 verdicts and step 2 handoff.
**Status:** Implemented 2026-11-08. Live-verified with an isolated `ORC_HOME` (routing table `debug → codex`, master opencode): run 1, no history → routed to codex (configured default) → real usage limit → handoff → opencode pass. Run 2, same kind of task → `route (debug): opencode — 1/1 passed · … · codex — rate-limited 0m ago` → opencode pass. `orc stats` showed opencode 2/2, $0.0168 total (cost from opencode's `step_finish`), codex 1 limit.

## Goal

orc picks which agent does a task, and in which order to hand off, from how agents actually did on that kind of task before, and shows why.

**Done when:** orc's routing choice changes because of its own run history, and the user can see why.

## Record

Every `orc run` attempt appends one line to `~/.orc/history.jsonl` (global, so learning spans repos; `ORC_HOME` overrides `~/.orc`):

`{ts, repo, run_id, attempt, category, provider, model, verdict, duration_ms, cost_usd}`

`cost_usd` is filled where the CLI reports it (Claude `result.total_cost_usd`, opencode `step_finish.part.cost`), otherwise null.

## Category

Cheap local keyword check on the task: `debug` (fix, bug, error, failing, broken, crash), `review` (review, audit), `plan` (plan, design, architecture, spec), `question` (starts with what/why/how/explain), else `implement`. `--category` overrides. No LLM call, so routing adds no cost or latency. The REPL's LLM classifier stays as is.

## Ranking

For each installed provider that can run tasks, in the task's category:

- **Score:** smoothed pass rate `(passes + 1) / (attempts + 2)`. Attempts that ended in `limit` don't count: a rate limit says nothing about skill.
- **Cold start:** the provider in the configured routing table for this category gets a stronger prior, `(passes + 2) / (attempts + 3)`, so with no history it ranks first, and real results override it after a few runs.
- **Cooldown:** a provider whose last attempt in any category was a `limit` within the past hour goes to the end.
- **Ties:** the master provider, then the `all_providers()` order.

The ranking sets both the first agent and the hand-off order.

## CLI

- `orc run` without `--provider`: routed. Prints e.g. `route (debug): opencode — 3/3 passed · then claude 1/2 · codex limited 12m ago`.
- `--provider X`: X goes first; fallbacks follow the ranking (unless `--fallback` is given).
- `--category`: override the keyword category.
- Model: `--model` for the first provider; otherwise the routing table's model if the first provider is the configured one.
- `orc stats [--category C] [--json]`: per category and provider: attempts, pass rate, limits, average duration, total cost.

## Code

| Where | Change |
|---|---|
| `orc-core/src/history.rs` (new) | `AttemptRecord`, append/load, `ProviderStats`, `stats_by_category` |
| `orc-core/src/route.rs` (new) | `categorize`, `rank` → `RoutePlan` with a reason per provider |
| `orc-core/src/config.rs` | `ORC_HOME` override |
| `orc-core/src/invoke` | `Provider::extract_cost`; `InvokeResult.cost_usd` |
| `orc-core/src/verify/stage.rs` | `StageSpec.category`, `history_path`; append a record after each verdict; `StageReport.cost_usd` |
| `orc-bin/src/main.rs` | routed `orc run`, `--category`, `orc stats` |

## Out of scope

LLM classification for routing; per-model stats (stats are per provider); using cost in the score (shown, not yet weighted); routing inside the REPL.
