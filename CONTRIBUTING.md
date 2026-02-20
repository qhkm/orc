# Contributing to orc

## Build & test

```bash
cargo build
cargo test
cargo install --path crates/orc-bin   # install to ~/.cargo/bin/orc
```

## Project layout

```
crates/
  orc-core/   # session parsers, SCP, flow compiler, provider invocation
  orc-tui/    # ratatui full-screen TUI (app.rs, widgets/)
  orc-bin/    # clap CLI entry point (main.rs)
```

---

## Session parser status

Each coding CLI stores sessions differently. `orc` discovers them all and shows
them in the TUI. Here's the current state — contributions welcome.

### ✅ Fully working

| Tool | Session path | cwd extracted? |
|------|-------------|----------------|
| **Claude** | `~/.claude/projects/**/*.jsonl` | ✅ Yes — from every user message |
| **Codex** | `~/.codex/sessions/*.jsonl` | ✅ Yes — from `session_meta.payload.cwd` |

### ⚠️ Works but cwd missing

Sessions are discovered and shown in the list, but they don't sort to the top
when you `cd` into their project because the format doesn't store a working
directory.

| Tool | Session path | Issue |
|------|-------------|-------|
| **Gemini** | `~/.gemini/tmp/<hash>/` | Dir names are SHA-256 hashes — can't decode to path |
| **Copilot** | `~/.copilot/session-state/` | `cwd` present only in some versions |
| **OpenCode** | `~/.local/share/opencode/opencode.db` | SQLite — cwd field may exist, needs verification |
| **Kimi** | `~/.kimi/sessions/` | Format is pure message log, no metadata fields at all |

**How to fix:** Add cwd extraction to the parser in
`crates/orc-core/src/session/<tool>.rs`. The `UnifiedSession` struct already
has a `cwd: PathBuf` field — just populate it. See `claude.rs` as the reference
implementation.

### 🚧 Stub — format unverified

These parsers were written as best-guess implementations without access to real
session files. They may parse incorrectly or not at all.

| Tool | Assumed path | What's needed |
|------|-------------|---------------|
| **Amp** | `~/.amp/sessions/` or `~/.config/amp/sessions/` | Real session file examples + correct format |
| **Droid** | `~/.factory/sessions/` | Real session file examples + correct format |

**How to fix:** If you have Amp or Droid installed, run:

```bash
ls ~/.amp/sessions/         # or ~/.config/amp/sessions/
cat ~/.amp/sessions/<file>  # see the real format
```

Then update `crates/orc-core/src/session/amp.rs` (or `droid.rs`) to match.
Include a test with a real-format fixture. The existing parser tests show the
pattern.

---

## Adding a new CLI

1. Create `crates/orc-core/src/session/<toolname>.rs`
2. Add a `SessionSource::<ToolName>` variant in `types.rs`
3. Add `files_modified: vec![]` to the `UnifiedSession` struct literal
4. Wire up discovery in `index.rs` → `build_index_full()`
5. Add display name in `app.rs` → `source_display_name()` and `ALL_SOURCES`
6. Add filter label in `widgets/footer.rs` → `filter_label()`

Reference: `crates/orc-core/src/session/claude.rs` is the most complete parser.

---

## Good first issues

- [ ] Extract cwd for Gemini sessions (decode the hash, or read from a metadata file inside the session dir)
- [ ] Verify and fix the Amp parser format
- [ ] Verify and fix the Droid parser format
- [ ] Extract cwd for Kimi (if a newer version of the CLI adds it)
- [ ] Extract `files_modified` for Codex sessions (scan for file write events)
- [ ] Add parser for a new coding CLI (Cursor, Windsurf, Aider, etc.)
