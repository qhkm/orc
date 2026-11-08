# orc

`orc` gets a coding task done correctly by whichever coding agents are best and available. It routes each request to the right agent CLI (`claude`, `codex`, `gemini`, `opencode`, `kimi` and others), carries context between them, and keeps a record of every step.

Direction and roadmap: [`docs/plans/2026-11-08-orc-positioning.md`](docs/plans/2026-11-08-orc-positioning.md). The loop is **route → run → verify → hand off on failure**. Routing, verification and automatic handoff (`orc run`), cross-tool session resume and the run log exist today; routing that learns from the run log is next.

## Install

```bash
cargo install --path crates/orc-bin   # installs orc to ~/.cargo/bin
orc init                              # detect installed coding CLIs, write ~/.orc/config.json
orc providers doctor                  # check each provider is installed and signed in
```

## Run a task and verify it

```bash
orc run "add auth middleware with tests" --provider claude \
  --check "cargo test" --check "cargo clippy -- -D warnings" \
  --accept "public functions have doc comments" --reviewer codex
```

The agent does the task, then orc decides whether it is done; the agent's own claim is not taken as the answer:

- `--check <cmd>` runs a shell command in the repo; it must exit 0. All checks run, and failing output becomes the evidence.
- `--accept <criterion>` is judged by a reviewer agent (`--reviewer`, default: master provider), which runs **read-only** and sees the diff since the run started.
- The agent is killed if it goes silent for `--idle` seconds (default 300) or runs past `--timeout` (default 1800).

| Verdict | Meaning | Exit code |
|---|---|---|
| `PASS` | Agent finished, every check passed, reviewer accepted every criterion | 0 |
| `FAIL` | Agent errored, a check failed, or the reviewer rejected a criterion (with evidence) | 1 |
| `STUCK` | Agent went silent or ran too long and was killed | 2 |
| `LIMIT` | Provider hit a rate limit or quota | 3 |
| | orc itself failed (bad arguments, unknown provider) | 4 |

### When an attempt does not pass

orc hands the task to another attempt instead of stopping:

```bash
orc run "fix the failing auth test" --provider codex --fallback claude --max-attempts 3 --check "cargo test"
```

```
▸ attempt 1 (codex): limit: codex: You've hit your usage limit…
▸ handing off codex → claude
▸ attempt 2 (claude): pass
PASS after 2 attempts (codex → claude)
```

| Verdict | Next attempt |
|---|---|
| `LIMIT` | next provider that hasn't hit a limit in this run |
| `STUCK` | another provider |
| `FAIL` | another provider; if there is none, the same one again with the failure as feedback |

The next agent gets the original task plus a handoff brief: the verdict, its evidence (failing check output, reviewer reasons, limit message), what the previous agent said it did, and what is already changed in the repo. The work is kept, not reset. The reviewer always judges the original task against the whole run's changes.

- `--fallback a,b`: hand-off order after `--provider` (default: every other installed provider that can run tasks). `--no-fallback`: only retry the same provider on `FAIL`.
- `--max-attempts N` (default 3). The exit code is the final attempt's verdict.
- `--model` applies to the first provider only, since model names differ per CLI.

`--json` prints the full report with every attempt. Every step, including each handoff, is logged to `.orc/runs/<run_id>/events.jsonl`.

### Supported agents

| Provider | Runs as | Reviewer | Notes |
|---|---|---|---|
| `claude` | `claude -p --output-format stream-json --verbose` | ✓ (Read/Grep/Glob only) | |
| `codex` | `codex exec --json --sandbox workspace-write` | ✓ (read-only sandbox) | |
| `opencode` | `opencode run --format json --auto` | ✓ (`--agent plan`) | `--model provider/model`, so it also runs DeepSeek, Qwen, local and other models |
| `pi` | `pi --print --mode json` | ✓ (`--no-tools`) | needs a provider/API key configured |
| `kimi` | `kimi --print --output-format stream-json --yolo` | | prints errors as plain text with exit 0; orc treats them as errors |
| `amp` | `amp --execute … --stream-json --dangerously-allow-all` | | `--model` is passed as amp's `--mode` |
| `gemini` | `gemini --yolo --prompt …` | | plain-text output (Gemini CLI 0.1.x has no JSON mode) |
| `copilot` | `copilot --prompt … --silent --allow-all-tools` | | plain-text output; pass `--model` if the default isn't on your plan |

A reviewer must enforce read-only itself, so only the four marked can judge `--accept`. `orc providers doctor` shows which are installed. Sign-in isn't checked up front: an attempt with a signed-out or out-of-quota CLI fails, and orc hands off to the next one.

## Use

```bash
# REPL: each request is classified (plan / implement / review / debug / question)
# and sent to the provider configured for that category
orc

# Sessions across all tools
orc list                       # every session from every supported tool
orc list --tool claude --format json
orc hub                        # TUI session browser

# Continue a session, in the same tool or a different one
orc resume <id>                # native resume
orc resume <id> --in codex     # cross-tool: Codex picks up with the Claude session's context

# Move context as a portable file
orc export <id>                # writes ./.orc/context.json
orc import ./.orc/context.json --in gemini

# Multi-stage flows from plain language
orc flow create my-default --prompt "claude brainstorms, then codex implements, then kimi cross-checks"
```

### REPL commands

| Command | Effect |
|---|---|
| `/set <category> <provider> [model]` | Change the routing for a category, e.g. `/set implement codex` |
| `/switch <provider>` | Send the next requests to one provider |
| `/budget [amount]` | Show or set the session cost budget |
| `/models`, `/providers`, `/sessions` | Show what's available |
| `/clear`, `/help` | |

Esc toggles `orc>` command mode. Shift+Tab switches to the next provider and hands it a brief of what was on screen, so it continues where the last one stopped.

## Routing

Routing lives in `~/.orc/config.json` under `routing`, mapping a category to a provider and optional model. Defaults: `plan` → Claude Opus, `implement` → Codex; unknown categories go to the master provider.

## Files

```
~/.orc/config.json                    # master provider, providers, routing
~/.orc/sessions.jsonl                 # session index cache
<repo>/.orc/flows/<name>.json         # flows
<repo>/.orc/runs/<id>/events.jsonl    # per-session event log
<repo>/.orc/context.json              # exported session context
```

## orc and herdr

[herdr](https://herdr.dev/) is a workspace for watching and steering many running agents. orc decides which agent does the work, checks whether it is done, and keeps the work moving when an agent fails. They are meant to be used together.

## Build and test

```bash
cargo build
cargo test
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for parser status and contribution guide.
