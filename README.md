# orc

`orc` gets a coding task done correctly by whichever coding agents are best and available. It routes each request to the right agent CLI (`claude`, `codex`, `gemini`, `opencode`, `kimi` and others), carries context between them, and keeps a record of every step.

Direction and roadmap: [`docs/plans/2026-11-08-orc-positioning.md`](docs/plans/2026-11-08-orc-positioning.md). The loop is **route → run → verify → hand off on failure**. Routing, cross-tool handoff and the run log exist today; verification and automatic handoff are next.

## Install

```bash
cargo install --path crates/orc-bin   # installs orc to ~/.cargo/bin
orc init                              # detect installed coding CLIs, write ~/.orc/config.json
orc providers doctor                  # check each provider is installed and signed in
```

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
