use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use crate::session::types::{SessionSource, UnifiedSession};

// ---------------------------------------------------------------------------
// Serde structs
//
// Factory Droid (the `droid` CLI) stores sessions under ~/.factory/sessions/.
// Each session is a directory containing:
//   history.jsonl     — the conversation log, one JSON object per line
//   .settings.json    — optional metadata (may include "cwd")
//
// Supported message formats in history.jsonl:
//
//   Simple format:
//     { "role": "user",      "content": "Add auth middleware" }
//     { "role": "assistant", "content": "I'll create..." }
//
//   Extended format with type field:
//     { "type": "message", "role": "user", "content": "..." }
//
//   Rich format (based on Factory Droid CLI actual JSONL format):
//     { "type": "session_start", "id": "...", "cwd": "/path/to/project", ... }
//     { "type": "message", "role": "user",      "content": "..." }  (string content)
//     { "type": "message", "role": "assistant", "content": "..." }
//
// Session ID: the directory name.
// CWD: from .settings.json "cwd" field, or from session_start event "cwd" field,
//      or falls back to the root directory.
// Summary: first user message (up to 80 chars).
// ---------------------------------------------------------------------------

/// Settings parsed from the optional `.settings.json` companion file.
#[derive(Debug, Deserialize, Default)]
struct DroidSettings {
    cwd: Option<String>,
}

/// A single event/message line from `history.jsonl`.
///
/// Handles:
///   - Simple: `{ "role": "user", "content": "..." }`
///   - Extended: `{ "type": "message", "role": "user", "content": "..." }`
///   - Session start: `{ "type": "session_start", "cwd": "..." }`
#[derive(Debug, Deserialize)]
struct DroidLine {
    #[serde(rename = "type")]
    line_type: Option<String>,
    role: Option<String>,
    /// Content as a plain string (simple / extended message format).
    content: Option<serde_json::Value>,
    /// CWD from a session_start event.
    cwd: Option<String>,
}

impl DroidLine {
    fn is_session_start(&self) -> bool {
        self.line_type.as_deref() == Some("session_start")
    }

    fn is_user_message(&self) -> bool {
        self.role.as_deref() == Some("user")
    }

    fn is_countable_message(&self) -> bool {
        matches!(self.role.as_deref(), Some("user") | Some("assistant"))
    }

    /// Extract text content from the line.
    fn text_content(&self) -> Option<String> {
        match &self.content {
            Some(serde_json::Value::String(s)) if !s.is_empty() => Some(s.clone()),
            _ => None,
        }
    }

    /// Extract CWD from a session_start event.
    fn session_cwd(&self) -> Option<PathBuf> {
        let raw = self.cwd.as_deref()?;
        if raw.is_empty() {
            None
        } else {
            Some(PathBuf::from(raw))
        }
    }
}

// ---------------------------------------------------------------------------
// Settings file parsing
// ---------------------------------------------------------------------------

/// Try to read `.settings.json` from the same directory as `history.jsonl`.
fn read_settings(session_dir: &Path) -> Option<DroidSettings> {
    let settings_path = session_dir.join(".settings.json");
    let content = std::fs::read_to_string(settings_path).ok()?;
    serde_json::from_str(&content).ok()
}

// ---------------------------------------------------------------------------
// JSONL parsing
// ---------------------------------------------------------------------------

/// Parse all lines from `history.jsonl`.
/// Returns `(lines, cwd_from_session_start)`.
fn parse_history_jsonl(file_path: &Path) -> Result<(Vec<DroidLine>, Option<PathBuf>)> {
    let file = std::fs::File::open(file_path)?;
    let reader = std::io::BufReader::new(file);
    let mut lines = Vec::new();
    let mut session_cwd: Option<PathBuf> = None;

    for raw in reader.lines() {
        let raw = raw?;
        if raw.trim().is_empty() {
            continue;
        }
        if let Ok(line) = serde_json::from_str::<DroidLine>(&raw) {
            if line.is_session_start() && session_cwd.is_none() {
                session_cwd = line.session_cwd();
            }
            lines.push(line);
        }
    }
    Ok((lines, session_cwd))
}

// ---------------------------------------------------------------------------
// Single session directory → UnifiedSession
// ---------------------------------------------------------------------------

fn session_from_dir(session_dir: &Path, fallback_cwd: &Path) -> Result<UnifiedSession> {
    let history_path = session_dir.join("history.jsonl");
    if !history_path.exists() {
        return Err(anyhow::anyhow!("no history.jsonl in {:?}", session_dir));
    }

    let (lines, session_start_cwd) = parse_history_jsonl(&history_path)?;

    // Count only user + assistant messages (not session_start or other events)
    let message_count = lines.iter().filter(|l| l.is_countable_message()).count();
    if message_count == 0 {
        return Err(anyhow::anyhow!(
            "no user/assistant messages in {:?}",
            session_dir
        ));
    }

    // Summary: first user message text, up to 80 chars
    let summary: String = lines
        .iter()
        .find(|l| l.is_user_message())
        .and_then(|l| l.text_content())
        .map(|text| {
            text.replace('\n', " ")
                .trim()
                .chars()
                .take(80)
                .collect()
        })
        .unwrap_or_default();

    // CWD precedence:
    //   1. .settings.json "cwd"
    //   2. session_start event "cwd"
    //   3. fallback_cwd (the root discovery directory)
    let settings = read_settings(session_dir);
    let cwd = settings
        .and_then(|s| s.cwd)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or(session_start_cwd)
        .unwrap_or_else(|| fallback_cwd.to_path_buf());

    // Session ID = directory name
    let session_id = session_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();

    // Timestamps from history.jsonl metadata
    let meta = std::fs::metadata(&history_path)?;
    let updated_at = meta
        .modified()
        .map(DateTime::<Utc>::from)
        .unwrap_or_else(|_| Utc::now());
    let created_at = meta
        .created()
        .map(DateTime::<Utc>::from)
        .unwrap_or(updated_at);

    Ok(UnifiedSession {
        id: session_id,
        source: SessionSource::Droid,
        cwd,
        repo: None,
        summary,
        updated_at,
        created_at,
        message_count,
        files_modified: vec![],
        original_path: history_path,
    })
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Discover and parse all Droid sessions under `root`.
/// `root` is typically `~/.factory/sessions/`.
/// Each subdirectory is treated as a session (must contain `history.jsonl`).
/// If `root` does not exist, returns an empty vec silently.
pub async fn discover_droid_sessions_in(root: &Path) -> Vec<UnifiedSession> {
    let mut sessions = Vec::new();

    if !root.exists() {
        return sessions;
    }

    let Ok(entries) = std::fs::read_dir(root) else {
        return sessions;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Ok(session) = session_from_dir(&path, root) {
                sessions.push(session);
            }
        }
    }

    sessions.sort_by_key(|s| std::cmp::Reverse(s.updated_at)); // newest first
    sessions
}

/// Discover all Droid sessions from the default location: `~/.factory/sessions/`.
pub async fn discover_droid_sessions() -> Vec<UnifiedSession> {
    let root = match dirs::home_dir() {
        Some(h) => h.join(".factory").join("sessions"),
        None => return Vec::new(),
    };
    discover_droid_sessions_in(&root).await
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    // ------------------------------------------------------------------
    // Helper: write a session directory with history.jsonl
    // ------------------------------------------------------------------

    fn write_session(root: &Path, session_id: &str, lines: &[&str]) -> PathBuf {
        let session_dir = root.join(session_id);
        fs::create_dir_all(&session_dir).unwrap();
        fs::write(session_dir.join("history.jsonl"), lines.join("\n")).unwrap();
        session_dir
    }

    fn write_settings(session_dir: &Path, cwd: &str) {
        let content = format!(r#"{{"cwd":"{}"}}"#, cwd);
        fs::write(session_dir.join(".settings.json"), content).unwrap();
    }

    // ------------------------------------------------------------------
    // Test 1: missing/empty root → empty vec
    // ------------------------------------------------------------------
    #[test]
    fn test_droid_empty_dir() {
        let tmp = TempDir::new().unwrap();
        let missing = tmp.path().join("does_not_exist");
        let rt = tokio::runtime::Runtime::new().unwrap();
        let sessions = rt.block_on(discover_droid_sessions_in(&missing));
        assert!(sessions.is_empty(), "Missing root should yield 0 sessions");
    }

    // ------------------------------------------------------------------
    // Test 2: empty root directory → empty vec
    // ------------------------------------------------------------------
    #[test]
    fn test_droid_root_exists_but_empty() {
        let tmp = TempDir::new().unwrap();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let sessions = rt.block_on(discover_droid_sessions_in(tmp.path()));
        assert!(sessions.is_empty(), "Empty root should yield 0 sessions");
    }

    // ------------------------------------------------------------------
    // Test 3: basic parse — user message → correct session
    // ------------------------------------------------------------------
    #[test]
    fn test_droid_basic_parse() {
        let tmp = TempDir::new().unwrap();
        write_session(
            tmp.path(),
            "session-abc",
            &[
                r#"{"role":"user","content":"Add auth middleware"}"#,
                r#"{"role":"assistant","content":"I'll create the middleware."}"#,
            ],
        );

        let rt = tokio::runtime::Runtime::new().unwrap();
        let sessions = rt.block_on(discover_droid_sessions_in(tmp.path()));
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].source, SessionSource::Droid);
        assert_eq!(sessions[0].id, "session-abc");
        assert!(
            sessions[0].summary.contains("Add auth middleware"),
            "Summary should be the first user message, got: {}",
            sessions[0].summary
        );
    }

    // ------------------------------------------------------------------
    // Test 4: message_count — N messages → correct count
    // ------------------------------------------------------------------
    #[test]
    fn test_droid_message_count() {
        let tmp = TempDir::new().unwrap();
        write_session(
            tmp.path(),
            "session-count",
            &[
                r#"{"role":"user","content":"Turn 1"}"#,
                r#"{"role":"assistant","content":"Turn 2"}"#,
                r#"{"role":"user","content":"Turn 3"}"#,
                r#"{"role":"assistant","content":"Turn 4"}"#,
            ],
        );

        let rt = tokio::runtime::Runtime::new().unwrap();
        let sessions = rt.block_on(discover_droid_sessions_in(tmp.path()));
        assert_eq!(sessions.len(), 1);
        assert_eq!(
            sessions[0].message_count, 4,
            "message_count should be 4, got: {}",
            sessions[0].message_count
        );
    }

    // ------------------------------------------------------------------
    // Test 5: .settings.json CWD extraction
    // ------------------------------------------------------------------
    #[test]
    fn test_droid_settings_cwd() {
        let tmp = TempDir::new().unwrap();
        let session_dir = write_session(
            tmp.path(),
            "session-cwd",
            &[
                r#"{"role":"user","content":"fix the bug"}"#,
                r#"{"role":"assistant","content":"Done."}"#,
            ],
        );
        write_settings(&session_dir, "/Users/me/myproject");

        let rt = tokio::runtime::Runtime::new().unwrap();
        let sessions = rt.block_on(discover_droid_sessions_in(tmp.path()));
        assert_eq!(sessions.len(), 1);
        assert_eq!(
            sessions[0].cwd,
            PathBuf::from("/Users/me/myproject"),
            "CWD should come from .settings.json"
        );
    }

    // ------------------------------------------------------------------
    // Test 6: session_start event CWD (when no .settings.json)
    // ------------------------------------------------------------------
    #[test]
    fn test_droid_session_start_cwd() {
        let tmp = TempDir::new().unwrap();
        write_session(
            tmp.path(),
            "session-start-cwd",
            &[
                r#"{"type":"session_start","id":"sess-001","cwd":"/home/dev/project"}"#,
                r#"{"role":"user","content":"implement feature"}"#,
                r#"{"role":"assistant","content":"Sure."}"#,
            ],
        );

        let rt = tokio::runtime::Runtime::new().unwrap();
        let sessions = rt.block_on(discover_droid_sessions_in(tmp.path()));
        assert_eq!(sessions.len(), 1);
        assert_eq!(
            sessions[0].cwd,
            PathBuf::from("/home/dev/project"),
            "CWD should come from session_start event"
        );
    }

    // ------------------------------------------------------------------
    // Test 7: session without history.jsonl is skipped
    // ------------------------------------------------------------------
    #[test]
    fn test_droid_dir_without_history() {
        let tmp = TempDir::new().unwrap();
        // Create a directory without history.jsonl
        fs::create_dir_all(tmp.path().join("session-no-history")).unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let sessions = rt.block_on(discover_droid_sessions_in(tmp.path()));
        assert!(sessions.is_empty(), "Directories without history.jsonl should be skipped");
    }

    // ------------------------------------------------------------------
    // Test 8: type=message format (extended format)
    // ------------------------------------------------------------------
    #[test]
    fn test_droid_type_message_format() {
        let tmp = TempDir::new().unwrap();
        write_session(
            tmp.path(),
            "session-extended",
            &[
                r#"{"type":"message","role":"user","content":"Build the API"}"#,
                r#"{"type":"message","role":"assistant","content":"Building now."}"#,
            ],
        );

        let rt = tokio::runtime::Runtime::new().unwrap();
        let sessions = rt.block_on(discover_droid_sessions_in(tmp.path()));
        assert_eq!(sessions.len(), 1);
        assert!(
            sessions[0].summary.contains("Build the API"),
            "Extended format should parse correctly; got: {}",
            sessions[0].summary
        );
        assert_eq!(sessions[0].message_count, 2);
    }

    // ------------------------------------------------------------------
    // Test 9: summary is capped at 80 chars
    // ------------------------------------------------------------------
    #[test]
    fn test_droid_summary_truncated() {
        let tmp = TempDir::new().unwrap();
        let long_msg = "x".repeat(200);
        let line = format!(r#"{{"role":"user","content":"{}"}}"#, long_msg);
        write_session(tmp.path(), "session-long", &[&line]);

        let rt = tokio::runtime::Runtime::new().unwrap();
        let sessions = rt.block_on(discover_droid_sessions_in(tmp.path()));
        assert_eq!(sessions.len(), 1);
        assert!(
            sessions[0].summary.len() <= 80,
            "Summary must be at most 80 chars, got: {}",
            sessions[0].summary.len()
        );
    }

    // ------------------------------------------------------------------
    // Bonus: .settings.json CWD takes precedence over session_start CWD
    // ------------------------------------------------------------------
    #[test]
    fn test_droid_settings_cwd_priority_over_session_start() {
        let tmp = TempDir::new().unwrap();
        let session_dir = write_session(
            tmp.path(),
            "session-priority",
            &[
                r#"{"type":"session_start","id":"s1","cwd":"/from/session_start"}"#,
                r#"{"role":"user","content":"test"}"#,
                r#"{"role":"assistant","content":"ok"}"#,
            ],
        );
        // .settings.json should win
        write_settings(&session_dir, "/from/settings");

        let rt = tokio::runtime::Runtime::new().unwrap();
        let sessions = rt.block_on(discover_droid_sessions_in(tmp.path()));
        assert_eq!(sessions.len(), 1);
        assert_eq!(
            sessions[0].cwd,
            PathBuf::from("/from/settings"),
            ".settings.json cwd should take priority"
        );
    }

    // ------------------------------------------------------------------
    // Bonus: multiple sessions sorted newest-first
    // ------------------------------------------------------------------
    #[tokio::test]
    async fn test_droid_sessions_sorted() {
        let tmp = TempDir::new().unwrap();
        let lines = &[
            r#"{"role":"user","content":"hello"}"#,
            r#"{"role":"assistant","content":"hi"}"#,
        ];
        write_session(tmp.path(), "session-alpha", lines);
        write_session(tmp.path(), "session-beta", lines);

        let sessions = discover_droid_sessions_in(tmp.path()).await;
        if sessions.len() >= 2 {
            assert!(sessions[0].updated_at >= sessions[1].updated_at);
        }
    }
}
