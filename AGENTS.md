# Repository Guidelines

## Purpose
`orc` routes a coding task to the best available agent CLI, verifies the result, and hands off to another agent on failure, keeping a record of each step. See `docs/plans/2026-11-08-orc-positioning.md`. It is not a terminal or workspace manager.

## Project Structure
Rust workspace (the original Go version is unmaintained and kept at the git tag `go-final`):
- `crates/orc-bin`: the `orc` binary and clap CLI (`src/main.rs`).
- `crates/orc-core`: config (`config.rs`), flows (`flow/`), provider invocation and cross-tool resume (`invoke/`), session parsers for amp, claude, codex, copilot, droid, gemini, kimi and opencode (`session/`).
- `crates/orc-repl`: REPL with classifier, router, cost tracking, slash commands and capsule event log.
- `crates/orc-tui`: ratatui session hub (`orc hub`).

Runtime files: `~/.orc/config.json`, `~/.orc/sessions.jsonl`, and repo-local `.orc/` (`flows/`, `runs/<id>/events.jsonl`, `context.json`).

## Build, Test, and Development Commands
- `cargo build`: build the workspace (on low-memory machines, `-j 2` keeps it light).
- `cargo test`: run all tests. `cargo test -p orc-core <name>` for a focused run.
- `cargo run -p orc -- <subcommand>`: run without installing.
- `cargo install --path crates/orc-bin`: install `orc`.
- `orc init` and `orc providers doctor`: check local provider setup after changes.

## Coding Style
- Run `cargo fmt` and `cargo clippy` before committing.
- One file per provider under `invoke/providers/` and `session/`; no provider-specific branches elsewhere.
- Keep config fields and JSON formats stable; add new fields as `Option` so old configs still load.

## Testing
- Unit tests in `#[cfg(test)] mod tests` at the bottom of each file.
- Cover parsers and command builders with fixture strings for each provider; provider CLIs change their output formats often.
- Use temp directories for filesystem tests; never touch the real `~/.orc`.

## Commits and PRs
Conventional Commits, e.g. `feat(repl): add /budget`, `fix(core): handle codex resume path`. PRs include a short problem/solution summary, `cargo test` output, and notes on any `.orc/` or config format change.

## Security
Do not commit secrets or machine-local config. `~/.orc/config.json` is user state. Commit only source and intentional fixtures.
