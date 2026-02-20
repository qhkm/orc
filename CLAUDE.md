# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build and Test

```bash
# Build the binary
go build ./cmd/orc

# Run all tests
go test ./...

# Run tests for a specific package
go test ./internal/flow/...
go test ./internal/cli/...

# Run a single test
go test ./internal/flow/ -run TestCompile

# Run the built binary
./orc
```

## Architecture Overview

`orc` is a Go CLI tool (`cmd/orc/main.go`) that orchestrates multi-agent coding workflows across provider CLIs (`claude`, `codex`, `kimi`, `opencode`).

### Request Flow

1. **Entry**: `cmd/orc/main.go` → `internal/cli.Run()` dispatches subcommands.
2. **Chat mode** (`orc` or `orc chat`): Loads a flow definition from `.orc/flows/<name>.json`, then for each user turn runs the flow's stages sequentially—each stage invokes one provider CLI via `internal/invoke`.
3. **Single invoke** (`orc invoke`): Directly runs one provider CLI via `internal/invoke` without a flow.
4. **Handoff** (`orc handoff`/`orc continue`): Creates persistent run records (`.orc/runs/<run_id>/handoff.json` + `brief.md`) that carry context between agents.

### Key Packages

| Package | Role |
|---|---|
| `internal/cli` | All CLI command handlers (`run.go`, `chat.go`), init wizard, flag parsing |
| `internal/invoke` | Builds and executes provider subprocess commands; parses JSONL responses from each provider; handles streaming |
| `internal/flow` | `Definition` model, `Compile()` (NL prompt → staged flow), `Save`/`Load` for `.orc/flows/*.json` |
| `internal/handoff` | `Service` creates/continues/loads run records; writes `handoff.json` + `brief.md` per run |
| `internal/providers` | `Provider` interface, `DoctorReport`, `registry.go` for detecting installed CLIs |
| `internal/doctor` | Runs `Doctor()` on each registered provider to check install + auth status |
| `internal/config` | Global config file (`~/.orc/config.json`) with master provider and registered providers |
| `internal/tui` | Terminal rendering: boxes, tables, colored badges, spinner/progress line |

### Flow Compilation

`flow.Compile()` parses a natural-language prompt to identify stage intents (`brainstorm`, `implement`, `cross_check`, `review`) using keyword matching. Provider assignment uses proximity-based matching between provider name aliases and intent keywords in the prompt. Stages chain via `InputFrom` pointing to the previous stage's ID.

### Provider Invocation

`invoke.Execute()` / `invoke.ExecuteStream()` build the exact subprocess command for each provider (e.g. `claude --dangerously-skip-permissions --output-format stream-json -p <prompt>`), run it with a timeout, and parse the JSONL or plain-text output back to a `Response` string. Each provider has a custom JSONL decoder (`parseClaudeJSONL`, `parseCodexJSONL`, `parseOpenCodeJSONL`).

### Scope Guard

In chat mode, `strictScope` (default `true`) gates every user message. A fast local heuristic (`isLikelyCodingOrRepoRequest`) is tried first; if ambiguous, the master provider is invoked to classify the request as `in_scope` or `out_of_scope` via a structured JSON prompt.

### Artifact Layout

```
~/.orc/config.json          # Global: master provider + registered providers
<repo>/.orc/flows/*.json    # Flow definitions
<repo>/.orc/runs/<id>/      # Per-run: handoff.json + brief.md
```

### Environment Variables

| Variable | Effect |
|---|---|
| `NO_COLOR` / `ORC_NO_COLOR=1` | Disable colored output |
| `ORC_NO_LIVE=1` | Disable live spinner |
| `KIMI_SHARE_DIR` | Override kimi share directory |
| `COLUMNS` | Override terminal width for panel wrapping |
