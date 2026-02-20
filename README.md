# orc

`orc` is a local-first coding-agent handoff orchestrator MVP.

## Commands

```bash
# Onboarding: detect installed coding CLIs and persist master config
orc init
orc init --interactive
orc init --master codex --json

# Provider capability and auth checks
orc providers doctor
orc providers doctor --provider codex --json

# TinyClaw-style direct provider invocation
orc invoke --provider claude --message "Create a 3-step plan for this repo" --timeout 90
orc invoke --provider codex --continue --message "Implement step 1 and summarize changes" --json
orc invoke --provider kimi --message "Cross-check for bugs and missing tests"
orc invoke --provider opencode --message "Alternative review pass with opencode"

# Coding-agent style interactive mode (single command, natural chat)
orc
orc --flow my-default --repo /path/to/repo
orc --flow my-default --repo /path/to/repo --once "add auth middleware with tests"
orc --flow my-default --once "..." --strict-scope=true --stream=true --live=true --show-raw=true --max-output-chars 12000
orc chat --flow my-default --repo /path/to/repo  # explicit alias
# If the requested flow is missing, `orc` falls back to the first `.orc/flows/*.json` file.
# Each stage prints observable details: provider command, duration, parsed response, and optional raw CLI output.
# Scope guard uses master agent first and blocks non-coding/non-repo prompts by default (`--strict-scope=true`).
# Stream stage output live with `--stream=true` (default); disable with `--stream=false`.
# Clean stream mode shows only meaningful agent text; use `--verbose=true` (or `/verbose`) to show raw stream details.
# Use `NO_COLOR=1` or `ORC_NO_COLOR=1` for plain, non-colored terminal output.
# Disable live spinner/status with `--live=false` or `ORC_NO_LIVE=1`.
# After each response in interactive mode, ORC prompts next actions: review, cross-check, refine implement, or continue.

# Compile a natural-language multi-agent flow
orc flow create \
  --name my-default \
  "i want claude code to brainstorm, then codex implement, then kimi cross-check, then review with opus"

# Create a handoff package
orc handoff \
  --from claude \
  --to codex \
  --goal "finish auth middleware and add tests" \
  --accept "all tests pass" \
  --accept "docs updated"

# Continue from an existing run
orc continue <run_id> --to opencode --json

# Load run state
orc status <run_id> --json
```

## Config and Artifacts

- Global config: `~/.orc/config.json`
- Flow specs: `<repo>/.orc/flows/<flow_name>.json`
- Run files: `<repo>/.orc/runs/<run_id>/`
  - `handoff.json` machine-readable context
  - `brief.md` human-readable handoff prompt

## Skills vs Orc

- `Skills`: reusable instruction packs for one agent session.
- `orc`: a multi-agent control plane that routes work across CLIs (`claude`, `codex`, `kimi`, `opencode`) with explicit handoffs.
- `Skills` optimize how a single agent behaves.
- `orc` optimizes who does each stage, in what order, with traceable artifacts.

Use both together:

1. Use skills inside each stage (brainstorm/implement/review quality).
2. Use `orc` to orchestrate stage sequencing, handoff, and run history.

## Build and test

```bash
go test ./...
go build ./cmd/orc
```
