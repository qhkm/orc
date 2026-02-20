# UX Improvements: Context-Aware Session Hub

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make `orc` feel like context follows you automatically — launch it from any project and instantly see what you were doing there, with richer info about what was touched.

**Architecture:** Three independent improvements: (1) cwd-aware startup sorting in the TUI, (2) files_modified added to UnifiedSession and extracted by Claude+Codex parsers, (3) auto-write SCP before every resume so `.orc/context.json` is always up-to-date.

**Tech Stack:** Rust, ratatui, orc-core, orc-tui, orc-bin

---

## Improvement 1: Auto-filter by cwd on startup

When `orc` is launched from inside a project directory, sessions from that directory float to the top of the list. The list title shows how many are local vs total.

---

### Task 1: Sort sessions by cwd match in SessionListWidget

**Files:**
- Modify: `crates/orc-tui/src/widgets/session_list.rs`

**Background:** `SessionListWidget` already has a `filtered_sessions()` method. We need to add a `project_cwd: Option<&'a Path>` field and sort matching sessions first.

**Step 1: Write the failing test**

Add to the test module at the bottom of `session_list.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use chrono::Utc;
    use orc_core::session::types::{SessionSource, UnifiedSession};

    fn make_session(id: &str, cwd: &str, source: SessionSource) -> UnifiedSession {
        UnifiedSession {
            id: id.to_string(),
            source,
            cwd: PathBuf::from(cwd),
            repo: None,
            summary: "test".to_string(),
            files_modified: vec![],
            updated_at: Utc::now(),
            created_at: Utc::now(),
            message_count: 1,
            original_path: PathBuf::from("/tmp/fake.jsonl"),
        }
    }

    #[test]
    fn test_project_cwd_sessions_sorted_first() {
        let sessions = vec![
            make_session("aaa", "/other/project", SessionSource::Claude),
            make_session("bbb", "/my/project", SessionSource::Claude),
            make_session("ccc", "/my/project", SessionSource::Codex),
        ];
        let widget = SessionListWidget {
            sessions: &sessions,
            filter: None,
            search: "",
            title: "test",
            project_cwd: Some(&PathBuf::from("/my/project")),
        };
        let filtered = widget.filtered_sessions();
        // bbb and ccc should come first (they match /my/project)
        assert_eq!(filtered[0].id, "bbb");
        assert_eq!(filtered[1].id, "ccc");
        assert_eq!(filtered[2].id, "aaa");
    }

    #[test]
    fn test_no_project_cwd_order_unchanged() {
        let sessions = vec![
            make_session("aaa", "/other", SessionSource::Claude),
            make_session("bbb", "/my/project", SessionSource::Claude),
        ];
        let widget = SessionListWidget {
            sessions: &sessions,
            filter: None,
            search: "",
            title: "test",
            project_cwd: None,
        };
        let filtered = widget.filtered_sessions();
        assert_eq!(filtered[0].id, "aaa");
        assert_eq!(filtered[1].id, "bbb");
    }
}
```

**Step 2: Run test to verify it fails**

```bash
cargo test --manifest-path ./Cargo.toml -p orc-tui 2>&1 | grep -E "FAILED|error"
```

Expected: compile error — `project_cwd` field doesn't exist yet.

**Step 3: Implement**

In `session_list.rs`:

1. Add `use std::path::Path;` to imports.

2. Add `project_cwd: Option<&'a Path>` field to `SessionListWidget`:

```rust
pub struct SessionListWidget<'a> {
    pub sessions: &'a [UnifiedSession],
    pub filter: Option<SessionSource>,
    pub search: &'a str,
    pub title: &'a str,
    pub project_cwd: Option<&'a Path>,   // NEW
}
```

3. Replace `filtered_sessions()` with a version that sorts matching sessions first:

```rust
pub fn filtered_sessions(&self) -> Vec<&UnifiedSession> {
    let mut matched: Vec<&UnifiedSession> = Vec::new();
    let mut rest: Vec<&UnifiedSession> = Vec::new();

    for s in self.sessions.iter() {
        // Source filter
        if let Some(f) = self.filter {
            if s.source != f { continue; }
        }
        // Search filter
        if !self.search.is_empty() {
            let q = self.search.to_lowercase();
            let hits = s.summary.to_lowercase().contains(&q)
                || s.cwd.to_string_lossy().to_lowercase().contains(&q)
                || s.id.to_lowercase().contains(&q);
            if !hits { continue; }
        }
        // CWD sort bucket
        if let Some(cwd) = self.project_cwd {
            if s.cwd == cwd {
                matched.push(s);
                continue;
            }
        }
        rest.push(s);
    }

    matched.extend(rest);
    matched
}
```

**Step 4: Run tests**

```bash
cargo test --manifest-path ./Cargo.toml -p orc-tui 2>&1 | tail -5
```

Expected: all tests pass.

---

### Task 2: Detect cwd in App and pass to SessionListWidget

**Files:**
- Modify: `crates/orc-tui/src/app.rs`

**Background:** `App` needs to detect `std::env::current_dir()` at startup and store it. Then pass it to `SessionListWidget` during render. Also update the list title to show local count.

**Step 1: Write the failing test**

In `app.rs`, add to the test module (or create one):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_local_session_count() {
        use orc_core::session::types::{SessionSource, UnifiedSession};
        use std::path::PathBuf;
        use chrono::Utc;

        let cwd = PathBuf::from("/my/project");
        let sessions = vec![
            UnifiedSession {
                id: "aaa".to_string(),
                source: SessionSource::Claude,
                cwd: PathBuf::from("/my/project"),
                repo: None,
                summary: "".to_string(),
                files_modified: vec![],
                updated_at: Utc::now(),
                created_at: Utc::now(),
                message_count: 0,
                original_path: PathBuf::from("/tmp/a.jsonl"),
            },
            UnifiedSession {
                id: "bbb".to_string(),
                source: SessionSource::Claude,
                cwd: PathBuf::from("/other"),
                repo: None,
                summary: "".to_string(),
                files_modified: vec![],
                updated_at: Utc::now(),
                created_at: Utc::now(),
                message_count: 0,
                original_path: PathBuf::from("/tmp/b.jsonl"),
            },
        ];
        let count = sessions.iter().filter(|s| s.cwd == cwd).count();
        assert_eq!(count, 1);
    }
}
```

**Step 2: Run to verify it compiles and passes (it should — it's a trivial count)**

```bash
cargo test --manifest-path ./Cargo.toml -p orc-tui test_local_session_count 2>&1 | tail -3
```

Note: This test will fail to compile until `files_modified` is added to `UnifiedSession` in Task 4. Come back to this after Task 4.

**Step 3: Implement**

In `app.rs`:

1. Add `project_cwd: Option<PathBuf>` field to `App` struct.

2. In `App::new()`, after loading sessions, set:
```rust
let project_cwd = std::env::current_dir().ok();
```
Store on `App`.

3. In the render function where `SessionListWidget` is constructed, pass:
```rust
SessionListWidget {
    sessions: &self.sessions,
    filter: self.filter,
    search: &self.search_query,
    title: &self.list_title(),   // new helper
    project_cwd: self.project_cwd.as_deref(),
}
```

4. Add `list_title()` helper method to `App`:
```rust
fn list_title(&self) -> String {
    let total = self.sessions.len();
    if let Some(cwd) = &self.project_cwd {
        let local = self.sessions.iter().filter(|s| s.cwd == *cwd).count();
        let project = cwd.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?");
        if local > 0 {
            return format!("{} — {} local, {} total", project, local, total);
        }
    }
    format!("Sessions ({})", total)
}
```

**Step 4: Run tests**

```bash
cargo test --manifest-path ./Cargo.toml 2>&1 | tail -5
```

Expected: all 158 tests still pass.

---

## Improvement 2: Richer session summaries (files touched)

Add `files_modified: Vec<String>` to `UnifiedSession`. Populate it in the Claude and Codex parsers. Display top 2 files in the session list row.

---

### Task 3: Add files_modified to UnifiedSession

**Files:**
- Modify: `crates/orc-core/src/session/types.rs`

**Background:** `UnifiedSession` needs a new `files_modified` field. This is a `Vec<String>` of file paths (relative or absolute) that were written/edited during the session. Populated by parsers; empty for parsers that don't support it yet.

**Step 1: Write the failing test**

In `types.rs` test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unified_session_files_modified_default_empty() {
        use std::path::PathBuf;
        use chrono::Utc;
        let s = UnifiedSession {
            id: "x".to_string(),
            source: SessionSource::Claude,
            cwd: PathBuf::from("/tmp"),
            repo: None,
            summary: "test".to_string(),
            files_modified: vec![],
            updated_at: Utc::now(),
            created_at: Utc::now(),
            message_count: 0,
            original_path: PathBuf::from("/tmp/x.jsonl"),
        };
        assert!(s.files_modified.is_empty());
    }

    #[test]
    fn test_unified_session_files_modified_roundtrip() {
        use std::path::PathBuf;
        use chrono::Utc;
        let s = UnifiedSession {
            id: "x".to_string(),
            source: SessionSource::Claude,
            cwd: PathBuf::from("/tmp"),
            repo: None,
            summary: "test".to_string(),
            files_modified: vec!["src/auth.ts".to_string(), "src/middleware.ts".to_string()],
            updated_at: Utc::now(),
            created_at: Utc::now(),
            message_count: 0,
            original_path: PathBuf::from("/tmp/x.jsonl"),
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: UnifiedSession = serde_json::from_str(&json).unwrap();
        assert_eq!(back.files_modified, vec!["src/auth.ts", "src/middleware.ts"]);
    }
}
```

**Step 2: Run to verify it fails**

```bash
cargo test --manifest-path ./Cargo.toml -p orc-core 2>&1 | grep "FAILED\|error\[" | head -5
```

Expected: compile error — `files_modified` not a field of `UnifiedSession`.

**Step 3: Implement**

In `types.rs`, add the field to `UnifiedSession` (after `summary`):

```rust
pub struct UnifiedSession {
    pub id: String,
    pub source: SessionSource,
    pub cwd: PathBuf,
    pub repo: Option<RepoState>,
    pub summary: String,
    #[serde(default)]
    pub files_modified: Vec<String>,   // NEW — top files written/edited this session
    pub updated_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub message_count: usize,
    pub original_path: PathBuf,
}
```

`#[serde(default)]` is important — it means old cached index files without this field will deserialize correctly (as empty vec).

**Step 4: Fix all compiler errors**

Every parser that constructs `UnifiedSession` using struct literal syntax will now fail to compile. Fix each by adding `files_modified: vec![]`:

Files to update:
- `crates/orc-core/src/session/claude.rs` — struct literal around line 168
- `crates/orc-core/src/session/codex.rs` — find struct literal
- `crates/orc-core/src/session/gemini.rs` — find struct literal
- `crates/orc-core/src/session/copilot.rs` — find struct literal
- `crates/orc-core/src/session/opencode.rs` — find struct literal
- `crates/orc-core/src/session/kimi.rs` — find struct literal
- `crates/orc-core/src/session/amp.rs` — find struct literal
- `crates/orc-core/src/session/droid.rs` — find struct literal
- `crates/orc-core/src/session/scp.rs` — `read_scp` builds a UnifiedSession
- `crates/orc-bin/src/main.rs` — `make_session()` helper in tests

Add `files_modified: vec![]` to each.

**Step 5: Run tests**

```bash
cargo test --manifest-path ./Cargo.toml 2>&1 | tail -5
```

Expected: all 158+ tests pass (plus 2 new).

---

### Task 4: Extract files_modified in Claude parser

**Files:**
- Modify: `crates/orc-core/src/session/claude.rs`

**Background:** The Claude parser currently reads only the first 50 lines to extract `summary`. We need to also scan the full file for `tool_use` blocks with `name` = "Write" or "Edit" and extract the `file_path` / `path` field from `input`. Cap at 5 unique paths. Filter out paths outside the session cwd.

**Step 1: Write the failing tests**

Add to `claude.rs` test module:

```rust
#[test]
fn test_extract_files_modified_from_write_tools() {
    use tempfile::TempDir;
    use std::io::Write;

    let dir = TempDir::new().unwrap();
    let proj = dir.path().join("projects").join("-tmp-myproject");
    std::fs::create_dir_all(&proj).unwrap();

    let session_id = "aaaaaaaa-0000-0000-0000-000000000000";
    let file = proj.join(format!("{}.jsonl", session_id));

    // Write a minimal JSONL with a Write tool_use
    let mut f = std::fs::File::create(&file).unwrap();
    writeln!(f, r#"{{"type":"summary","sessionId":"{}","cwd":"/tmp/myproject"}}"#, session_id).unwrap();
    writeln!(f, r#"{{"type":"user","uuid":"u1","timestamp":"2026-01-01T00:00:00Z","message":{{"role":"user","content":"add auth"}}}}"#).unwrap();
    writeln!(f, r#"{{"type":"assistant","uuid":"a1","timestamp":"2026-01-01T00:00:01Z","message":{{"role":"assistant","content":[{{"type":"tool_use","id":"t1","name":"Write","input":{{"file_path":"/tmp/myproject/src/auth.ts","content":"..."}}}}]}}}}"#).unwrap();
    writeln!(f, r#"{{"type":"assistant","uuid":"a2","timestamp":"2026-01-01T00:00:02Z","message":{{"role":"assistant","content":[{{"type":"tool_use","id":"t2","name":"Edit","input":{{"file_path":"/tmp/myproject/src/middleware.ts","old_string":"x","new_string":"y"}}}}]}}}}"#).unwrap();

    let sessions = tokio::runtime::Builder::new_current_thread()
        .enable_all().build().unwrap()
        .block_on(discover_claude_sessions_in(dir.path().join("projects")));

    assert_eq!(sessions.len(), 1);
    let s = &sessions[0];
    assert!(s.files_modified.contains(&"src/auth.ts".to_string()) ||
            s.files_modified.contains(&"/tmp/myproject/src/auth.ts".to_string()),
            "expected auth.ts in files_modified, got: {:?}", s.files_modified);
    assert!(s.files_modified.len() <= 5);
}

#[test]
fn test_files_modified_capped_at_five() {
    // Build a session with 8 Write tool calls → expect at most 5 unique paths
    use tempfile::TempDir;
    use std::io::Write;

    let dir = TempDir::new().unwrap();
    let proj = dir.path().join("projects").join("-tmp-cap");
    std::fs::create_dir_all(&proj).unwrap();
    let session_id = "bbbbbbbb-0000-0000-0000-000000000000";
    let file = proj.join(format!("{}.jsonl", session_id));

    let mut f = std::fs::File::create(&file).unwrap();
    writeln!(f, r#"{{"type":"summary","sessionId":"{}","cwd":"/tmp/cap"}}"#, session_id).unwrap();
    writeln!(f, r#"{{"type":"user","uuid":"u1","timestamp":"2026-01-01T00:00:00Z","message":{{"role":"user","content":"do stuff"}}}}"#).unwrap();
    for i in 0..8usize {
        writeln!(f, r#"{{"type":"assistant","uuid":"a{}","timestamp":"2026-01-01T00:00:0{}Z","message":{{"role":"assistant","content":[{{"type":"tool_use","id":"t{}","name":"Write","input":{{"file_path":"/tmp/cap/file{}.ts","content":"..."}}}}]}}}}"#, i, i, i, i).unwrap();
    }

    let sessions = tokio::runtime::Builder::new_current_thread()
        .enable_all().build().unwrap()
        .block_on(discover_claude_sessions_in(dir.path().join("projects")));

    assert_eq!(sessions.len(), 1);
    assert!(sessions[0].files_modified.len() <= 5,
        "files_modified should be capped at 5, got {}", sessions[0].files_modified.len());
}
```

**Step 2: Run to verify tests fail**

```bash
cargo test --manifest-path ./Cargo.toml -p orc-core test_extract_files_modified 2>&1 | tail -5
```

Expected: FAIL — files_modified is empty.

**Step 3: Implement**

In `claude.rs`, add a `extract_files_modified(lines: &[String], cwd: &Path) -> Vec<String>` function:

```rust
/// Scan raw JSONL lines for Write/Edit tool_use blocks.
/// Returns up to 5 unique written/edited file paths (relative to cwd when possible).
fn extract_files_modified(lines: &[String], cwd: &std::path::Path) -> Vec<String> {
    const WRITE_TOOLS: &[&str] = &["Write", "Edit", "MultiEdit", "WriteFile", "EditFile"];
    let mut seen = std::collections::LinkedHashSet::new(); // ordered, dedup
    // Use IndexSet or just a Vec + contains check for simplicity:
    let mut files: Vec<String> = Vec::new();

    for line in lines {
        if files.len() >= 5 { break; }
        let Ok(val) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        let Some(content) = val.get("message").and_then(|m| m.get("content")).and_then(|c| c.as_array()) else { continue };
        for item in content {
            if files.len() >= 5 { break; }
            let Some(name) = item.get("name").and_then(|n| n.as_str()) else { continue };
            if !WRITE_TOOLS.contains(&name) { continue; }
            let input = item.get("input").and_then(|i| i.as_object());
            let path_str = input
                .and_then(|m| m.get("file_path").or_else(|| m.get("path")))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if path_str.is_empty() { continue; }
            // Make relative to cwd if possible
            let display = std::path::Path::new(path_str)
                .strip_prefix(cwd)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| path_str.to_string());
            if !files.contains(&display) {
                files.push(display);
            }
        }
    }
    files
}
```

Then in `parseClaudeSessions()` (the `for filePath in files` loop), after getting `info`, read all lines and call `extract_files_modified`:

```rust
// Read all raw lines for files_modified extraction
let raw_lines = read_raw_lines(&file_path).await;  // new helper (see below)
let cwd_path = std::path::Path::new(&info.cwd);
let files_modified = extract_files_modified(&raw_lines, cwd_path);
```

Add `read_raw_lines` helper (reads all lines as Strings, similar to `readAllMessages` but cheaper):

```rust
async fn read_raw_lines(path: &std::path::Path) -> Vec<String> {
    tokio::task::spawn_blocking({
        let path = path.to_path_buf();
        move || {
            use std::io::BufRead;
            let file = std::fs::File::open(&path).ok()?;
            let reader = std::io::BufReader::new(file);
            Some(reader.lines().filter_map(|l| l.ok()).collect::<Vec<_>>())
        }
    })
    .await
    .ok()
    .flatten()
    .unwrap_or_default()
}
```

Set `files_modified` in the `UnifiedSession` struct literal.

**Step 4: Run tests**

```bash
cargo test --manifest-path ./Cargo.toml -p orc-core 2>&1 | tail -5
```

Expected: all tests pass including the 2 new ones.

---

### Task 5: Show files_modified in session list row

**Files:**
- Modify: `crates/orc-tui/src/widgets/session_list.rs`

**Background:** The current row format is `[badge]  time  project  summary...  id`. We want to add a files column showing up to 2 file paths (basename only) when available.

**Step 1: Write the failing test**

```rust
#[test]
fn test_row_shows_files_when_present() {
    use std::path::PathBuf;
    use chrono::Utc;
    use orc_core::session::types::{SessionSource, UnifiedSession};

    let session = UnifiedSession {
        id: "abc12345".to_string(),
        source: SessionSource::Claude,
        cwd: PathBuf::from("/my/project"),
        repo: None,
        summary: "add auth".to_string(),
        files_modified: vec!["src/auth.ts".to_string(), "src/middleware.ts".to_string()],
        updated_at: Utc::now(),
        created_at: Utc::now(),
        message_count: 5,
        original_path: PathBuf::from("/tmp/abc.jsonl"),
    };

    let files_str = files_badge(&session);
    assert!(files_str.contains("auth.ts"), "got: {}", files_str);
}

#[test]
fn test_row_shows_empty_when_no_files() {
    use std::path::PathBuf;
    use chrono::Utc;
    use orc_core::session::types::{SessionSource, UnifiedSession};

    let session = UnifiedSession {
        id: "abc12345".to_string(),
        source: SessionSource::Claude,
        cwd: PathBuf::from("/my/project"),
        repo: None,
        summary: "add auth".to_string(),
        files_modified: vec![],
        updated_at: Utc::now(),
        created_at: Utc::now(),
        message_count: 5,
        original_path: PathBuf::from("/tmp/abc.jsonl"),
    };

    let files_str = files_badge(&session);
    assert!(files_str.is_empty(), "got: {}", files_str);
}
```

**Step 2: Run to verify compile failure**

Expected: `files_badge` not defined.

**Step 3: Implement**

Add `files_badge` function in `session_list.rs`:

```rust
/// Returns a compact files string: "auth.ts  middleware.ts" (basenames, max 2 files).
fn files_badge(session: &UnifiedSession) -> String {
    let top: Vec<String> = session.files_modified
        .iter()
        .take(2)
        .map(|p| {
            std::path::Path::new(p)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(p.as_str())
                .to_string()
        })
        .collect();
    top.join("  ")
}
```

Then in the `render` method, update the `ListItem` line to include files after summary:

```rust
let files = files_badge(session);
let line = Line::from(vec![
    Span::styled(format!("{:<12}", badge), badge_style(src_key)),
    Span::raw("  "),
    Span::styled(format!("{:<10}", time), Style::default().fg(Color::DarkGray)),
    Span::raw("  "),
    Span::styled(format!("{:<16}", project), Style::default().fg(Color::Cyan)),
    Span::raw("  "),
    Span::styled(format!("{:<36}", summary), Style::default()),
    Span::raw("  "),
    Span::styled(format!("{:<28}", files), Style::default().fg(Color::DarkGray)),  // NEW
    Span::raw("  "),
    Span::styled(id.to_string(), Style::default().fg(Color::DarkGray)),
]);
```

**Step 4: Run tests**

```bash
cargo test --manifest-path ./Cargo.toml 2>&1 | tail -5
```

Expected: all tests pass.

---

## Improvement 3: SCP auto-write on resume

Before every resume (TUI Enter key, headless `orc resume`), automatically write `.orc/context.json` in the session's project directory. This means any tool opening that project later will find up-to-date context.

---

### Task 6: Auto-write SCP before TUI resume

**Files:**
- Modify: `crates/orc-tui/src/app.rs`

**Background:** In `app.rs`, when the user presses Enter in `SessionHub` view, the code calls `spawn_and_exit(cmd)`. Before that call, we write the SCP. We already have `make_minimal_context()` which builds a `SessionContext` from a `UnifiedSession`. We call `write_scp(path, &ctx)` with path = `<session.cwd>/.orc/context.json`.

**Step 1: Write the failing test**

In `app.rs` test module:

```rust
#[test]
fn test_scp_path_from_session() {
    use std::path::PathBuf;
    use orc_core::session::scp::default_scp_path;
    let cwd = PathBuf::from("/my/project");
    let path = default_scp_path(&cwd);
    assert_eq!(path, PathBuf::from("/my/project/.orc/context.json"));
}
```

**Step 2: Run to verify — should already pass** (tests `default_scp_path` which exists in `scp.rs`)

```bash
cargo test --manifest-path ./Cargo.toml test_scp_path_from_session 2>&1 | tail -3
```

**Step 3: Implement**

In `app.rs`, find the Enter key handler that calls `spawn_and_exit`. Before the `spawn_and_exit` call, add SCP auto-write:

```rust
// Auto-write SCP before resume so other tools find context
if let Some(session) = self.selected_session().cloned() {
    let scp_path = orc_core::session::scp::default_scp_path(&session.cwd);
    let ctx = make_minimal_context(&session);
    let _ = orc_core::session::scp::write_scp(&scp_path, &ctx); // silent fail — non-critical

    // Now resume
    let cmd = build_resume_command(session.source, &session.id, &session.original_path, &session.cwd);
    // leave alternate screen before exec
    let _ = disable_raw_mode();
    let _ = execute!(stdout(), LeaveAlternateScreen);
    spawn_and_exit(cmd);
}
```

Do the same for cross-tool resume (the `r` key + target selection path).

**Step 4: Run tests**

```bash
cargo test --manifest-path ./Cargo.toml 2>&1 | tail -5
```

Expected: all tests pass.

---

### Task 7: Auto-write SCP in headless resume command

**Files:**
- Modify: `crates/orc-bin/src/main.rs`

**Background:** `cmd_resume` in `main.rs` calls `spawn_and_exit` after finding the session. Add the same SCP auto-write before it.

**Step 1: Write the test**

```rust
#[test]
fn test_cmd_resume_would_write_scp_path() {
    use orc_core::session::scp::default_scp_path;
    use std::path::PathBuf;
    // Verify the path we'd write to is sensible
    let cwd = PathBuf::from("/Users/me/myproject");
    let path = default_scp_path(&cwd);
    assert!(path.to_string_lossy().ends_with(".orc/context.json"));
    assert!(path.to_string_lossy().contains("myproject"));
}
```

**Step 2: Run to verify it passes** (trivial test, should pass immediately)

**Step 3: Implement**

In `main.rs`, find `cmd_resume`. Before the `spawn_and_exit` call:

```rust
// Auto-write SCP
let scp_path = orc_core::session::scp::default_scp_path(&session.cwd);
let ctx = orc_core::session::types::SessionContext {
    session: session.clone(),
    messages: vec![],
    tool_activity: vec![],
    files_modified: session.files_modified.clone(),
    reasoning_notes: vec![],
    pending_tasks: vec![],
    token_usage: None,
    markdown: String::new(),
};
let _ = orc_core::session::scp::write_scp(&scp_path, &ctx);
```

**Step 4: Run all tests**

```bash
cargo test --manifest-path ./Cargo.toml 2>&1 | tail -8
```

Expected: all 158+ tests pass.

---

## Final verification

**Step 1: Build release binary**

```bash
cargo build --manifest-path ./Cargo.toml 2>&1 | tail -3
```

**Step 2: Test headless commands**

```bash
cd .
./target/debug/orc list | head -5
# Should show: "orc — X local, Y total" in the list header (visible in TUI, not headless)

./target/debug/orc list --format json | python3 -c "import sys,json; d=json.load(sys.stdin); print(d[0].get('files_modified','MISSING'))"
# Should show: [] or a list of files (not "MISSING")
```

**Step 3: Verify SCP auto-write**

```bash
./target/debug/orc export 3be345bb
cat ./.orc/context.json | python3 -m json.tool | head -10
```

**Step 4: Run full test suite one final time**

```bash
cargo test --manifest-path ./Cargo.toml 2>&1 | grep "test result"
```

Expected:
```
test result: ok. N passed; 0 failed
test result: ok. N passed; 0 failed
test result: ok. N passed; 0 failed
```

---

## Improvement 4: "What next?" handoff popup on Enter

Instead of Enter immediately launching the tool, show a one-step popup that lets the user decide: continue in the same tool, or hand off to another. This mirrors the natural "which approach?" moment at the end of a planning session.

---

### Task 8: Replace direct resume with "what next?" popup

**Files:**
- Modify: `crates/orc-tui/src/app.rs`

**Background:** Currently pressing Enter on a session immediately calls `spawn_and_exit`. Replace this with a small centered popup showing 4 choices. The user picks with number keys 1–4 or arrow + Enter. Pressing Esc cancels back to the list.

**What the popup looks like:**

```
┌─ Resume session ──────────────────────────────────┐
│                                                    │
│  add auth middleware · ios/orc · 2h ago            │
│                                                    │
│  1  Continue in claude     (same tool)             │
│  2  Hand off to codex                              │
│  3  Hand off to gemini                             │
│  4  Hand off to…           (pick any tool)         │
│                                                    │
│  Esc  cancel                                       │
└────────────────────────────────────────────────────┘
```

Options 1–3 are: same tool, then the top 2 *other* installed tools (from `providers doctor` order: codex, gemini, copilot, opencode).
Option 4 opens the existing cross-tool picker (`r` key behavior).

**Step 1: Write the failing test**

In `app.rs` tests:

```rust
#[test]
fn test_next_popup_options_same_tool_first() {
    // Given a Claude session, option 1 should be "claude"
    use orc_core::session::types::SessionSource;
    let source = SessionSource::Claude;
    let opts = next_popup_options(source);
    assert_eq!(opts[0], SessionSource::Claude);
    assert!(opts.len() >= 2);
    // All options are distinct
    let unique: std::collections::HashSet<_> = opts.iter().collect();
    assert_eq!(unique.len(), opts.len());
}

#[test]
fn test_next_popup_options_excludes_unknown() {
    use orc_core::session::types::SessionSource;
    let opts = next_popup_options(SessionSource::Codex);
    // codex should be first
    assert_eq!(opts[0], SessionSource::Codex);
    // No Scp or unknown sources
    assert!(!opts.contains(&SessionSource::Scp));
}
```

**Step 2: Run to verify compile failure**

Expected: `next_popup_options` not defined.

**Step 3: Implement**

Add a new `AppMode` variant or a new popup state. The simplest approach: add a `next_popup: Option<NextPopupState>` field to `App`.

```rust
pub struct NextPopupState {
    pub session: UnifiedSession,
    pub options: Vec<SessionSource>,  // [same_tool, alt1, alt2, "pick…"]
    pub selected: usize,
}
```

Add `next_popup_options(source: SessionSource) -> Vec<SessionSource>` function:

```rust
/// Returns resume options: [source itself, top 2 other common tools].
/// Used by the "what next?" popup.
pub fn next_popup_options(source: SessionSource) -> Vec<SessionSource> {
    const COMMON: &[SessionSource] = &[
        SessionSource::Claude,
        SessionSource::Codex,
        SessionSource::Gemini,
        SessionSource::Copilot,
        SessionSource::OpenCode,
    ];
    let mut opts = vec![source];
    for &s in COMMON {
        if s != source && opts.len() < 3 {
            opts.push(s);
        }
    }
    opts
}
```

Change the Enter key handler:

```rust
// Instead of immediately resuming, open "what next?" popup
KeyCode::Enter if self.next_popup.is_none() => {
    if let Some(session) = self.selected_session().cloned() {
        let options = next_popup_options(session.source);
        self.next_popup = Some(NextPopupState {
            session,
            options,
            selected: 0,
        });
    }
}
```

Add popup event handling (when `self.next_popup.is_some()`):

```rust
// Number keys 1–3 select option directly
KeyCode::Char('1') | KeyCode::Char('2') | KeyCode::Char('3') => {
    if let Some(popup) = &self.next_popup {
        let idx = (key.code as u8 - b'1') as usize;
        if idx < popup.options.len() {
            let session = popup.session.clone();
            let target = popup.options[idx];
            self.next_popup = None;
            self.do_resume(session, target);  // see below
        }
    }
}
// Esc cancels
KeyCode::Esc if self.next_popup.is_some() => {
    self.next_popup = None;
}
// Up/Down navigate
KeyCode::Up if self.next_popup.is_some() => { /* decrement selected */ }
KeyCode::Down if self.next_popup.is_some() => { /* increment selected */ }
// Enter confirms selected
KeyCode::Enter if self.next_popup.is_some() => {
    if let Some(popup) = &self.next_popup {
        let session = popup.session.clone();
        let target = popup.options[popup.selected];
        self.next_popup = None;
        self.do_resume(session, target);
    }
}
```

Add `do_resume(session, target)` method that writes SCP then calls `spawn_and_exit` (same-tool) or `cross_tool_resume` (different tool):

```rust
fn do_resume(&mut self, session: UnifiedSession, target: SessionSource) {
    // Auto-write SCP
    let scp_path = orc_core::session::scp::default_scp_path(&session.cwd);
    let ctx = make_minimal_context(&session);
    let _ = orc_core::session::scp::write_scp(&scp_path, &ctx);

    let _ = disable_raw_mode();
    let _ = execute!(stdout(), LeaveAlternateScreen);

    if target == session.source {
        let cmd = build_resume_command(session.source, &session.id, &session.original_path, &session.cwd);
        spawn_and_exit(cmd);
    } else {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let _ = rt.block_on(cross_tool_resume(&ctx, target));
    }
}
```

Add popup rendering in the render function:

```rust
if let Some(popup) = &self.next_popup {
    let title = format!(" {} · {} · {} ",
        popup.session.summary.chars().take(40).collect::<String>(),
        popup.session.cwd.file_name().and_then(|n| n.to_str()).unwrap_or("?"),
        format_time_ago_session(&popup.session),
    );
    let lines: Vec<Line> = popup.options.iter().enumerate().map(|(i, src)| {
        let label = source_display_name(*src);
        let hint = if *src == popup.session.source { " (same tool)" } else { "" };
        let text = format!("  {}  {}{}", i + 1, label, hint);
        if i == popup.selected {
            Line::from(Span::styled(text, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)))
        } else {
            Line::from(Span::raw(text))
        }
    }).collect();

    let popup_area = centered_rect(52, (popup.options.len() + 4) as u16, area);
    frame.render_widget(Clear, popup_area);
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title(title)),
        popup_area,
    );
}
```

**Step 4: Remove the old cross-tool `r` popup** (or keep it as a fast shortcut — your call). Keeping it is fine.

**Step 5: Run tests**

```bash
cargo test --manifest-path ./Cargo.toml 2>&1 | tail -5
```

Expected: all tests pass including 2 new popup tests.

**Step 6: Build and smoke-test manually**

```bash
cargo build --manifest-path ./Cargo.toml
```

Then open a terminal and run `./target/debug/orc`. Press Enter on any session — the popup should appear with 3 tool options. Press `1` to resume in the same tool, or `2`/`3` to hand off.
