# Repository Guidelines

## Project Structure & Module Organization
`orc` is a Go CLI entrypoint at `cmd/orc/main.go`. Core logic lives in `internal/`:
- `internal/cli`: command parsing and interactive chat flow.
- `internal/providers`: provider adapters and process runners.
- `internal/flow`, `internal/handoff`, `internal/invoke`: orchestration and handoff state.
- `internal/config`, `internal/doctor`, `internal/tui`: config, diagnostics, and terminal UI.

Tests are colocated with source files as `*_test.go` (for example, `internal/cli/chat_test.go`). Runtime artifacts are written under repo-local `.orc/` (`.orc/flows`, `.orc/runs`).

## Build, Test, and Development Commands
- `go test ./...`: run all unit tests across packages.
- `go test ./internal/cli -run TestResolveChatFlowPath`: run a focused test during iteration.
- `go build ./cmd/orc`: compile the CLI binary.
- `go run ./cmd/orc --help`: run without a manual build.
- `orc init` and `orc providers doctor`: verify local provider setup after changes.

## Coding Style & Naming Conventions
Use standard Go formatting and idioms:
- Run `gofmt -w .` before opening a PR.
- Keep package names short and lowercase (`cli`, `flow`, `tui`).
- Use `camelCase` for unexported identifiers and `PascalCase` for exported types/functions.
- Prefer table-driven tests for behavior matrices and edge cases.
- Keep CLI flags and JSON fields explicit and stable (see `internal/cli` and `internal/config`).

## Testing Guidelines
Use Go’s `testing` package with deterministic tests:
- Name tests `TestXxx` and colocate them with the code under test.
- Use `t.Parallel()` for independent tests where safe.
- Use `t.TempDir()` and explicit fixture creation for filesystem behavior.
- Cover both success and failure paths when adding commands or provider integrations.

## Commit & Pull Request Guidelines
No `.git` history is available in this workspace snapshot, so follow Conventional Commit style:
- `feat(cli): add follow-up shortcut`
- `fix(providers): handle timeout exit code`
- `test(flow): cover fallback flow selection`

PRs should include:
- A short problem/solution summary.
- Linked issue (if available).
- Evidence of verification (`go test ./...`, command output, or screenshots for terminal UX changes).
- Notes on config/artifact impacts (especially `.orc/` behavior).

## Security & Configuration Tips
Do not commit secrets or machine-local config. Keep provider auth tokens in environment-specific locations and treat `~/.orc/config.json` as local user state. Commit only source and intentional fixture data.
