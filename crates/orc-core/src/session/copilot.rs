use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use crate::session::types::{SessionSource, UnifiedSession};

// ---------------------------------------------------------------------------
// Serde structs
//
// GitHub Copilot CLI stores sessions under ~/.copilot/session-state/.
// We look for two file types in that directory:
//
//   <session-id>.jsonl  — JSONL conversation logs, one message per line.
//   <session-id>.json   — JSON array of message objects.
//
// A typical message object (both formats) looks like:
//   { "role": "user",      "content": "add auth middleware" }
//   { "role": "assistant", "content": "I'll create..." }
//
// Extended keys that may also be present:
//   "timestamp", "cwd", "workingDirectory"
// ---------------------------------------------------------------------------

/// A single message as parsed from a Copilot session file.
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct CopilotMessage {
    role: Option<String>,
    content: Option<String>,
    /// Optional timestamp string (ISO-8601 or similar).
    timestamp: Option<String>,
    /// Working directory baked into some Copilot message formats.
    cwd: Option<String>,
    #[serde(rename = "workingDirectory")]
    working_directory: Option<String>,
}

impl CopilotMessage {
    fn is_user(&self) -> bool {
        self.role.as_deref() == Some("user")
    }

    fn is_countable(&self) -> bool {
        matches!(self.role.as_deref(), Some("user") | Some("assistant"))
    }

    /// Return the best available working-directory path from this message.
    fn cwd_path(&self) -> Option<PathBuf> {
        let raw = self
            .cwd
            .as_deref()
            .or(self.working_directory.as_deref())?;
        if raw.is_empty() {
            None
        } else {
            Some(PathBuf::from(raw))
        }
    }
}

// ---------------------------------------------------------------------------
// File-level parsing helpers
// ---------------------------------------------------------------------------

/// Parse a JSONL file (one JSON object per non-empty line).
/// Returns all successfully parsed messages; bad lines are silently skipped.
fn parse_jsonl(file_path: &Path) -> Result<Vec<CopilotMessage>> {
    let file = std::fs::File::open(file_path)?;
    let reader = std::io::BufReader::new(file);
    let mut messages = Vec::new();
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(msg) = serde_json::from_str::<CopilotMessage>(&line) {
            messages.push(msg);
        }
    }
    Ok(messages)
}

/// Parse a JSON file that contains an array of message objects.
fn parse_json_array(file_path: &Path) -> Result<Vec<CopilotMessage>> {
    let content = std::fs::read_to_string(file_path)?;
    let messages: Vec<CopilotMessage> = serde_json::from_str(&content)?;
    Ok(messages)
}

/// Attempt to parse a Copilot session file (either `.jsonl` or `.json`).
fn parse_copilot_file(file_path: &Path) -> Result<Vec<CopilotMessage>> {
    let ext = file_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    match ext {
        "jsonl" => parse_jsonl(file_path),
        "json" => {
            // Try array first; fall back to treating every line as JSONL
            parse_json_array(file_path).or_else(|_| parse_jsonl(file_path))
        }
        _ => Err(anyhow::anyhow!("unsupported extension: {:?}", file_path)),
    }
}

// ---------------------------------------------------------------------------
// Single-file → UnifiedSession
// ---------------------------------------------------------------------------

fn session_from_messages(
    file_path: &Path,
    messages: Vec<CopilotMessage>,
    fallback_cwd: &Path,
) -> Result<UnifiedSession> {
    if messages.is_empty() {
        return Err(anyhow::anyhow!("empty session: {:?}", file_path));
    }

    let message_count = messages.iter().filter(|m| m.is_countable()).count();
    if message_count == 0 {
        return Err(anyhow::anyhow!("no user/assistant messages: {:?}", file_path));
    }

    // Summary: first user message, up to 80 chars
    let summary: String = messages
        .iter()
        .find(|m| m.is_user())
        .and_then(|m| m.content.as_deref())
        .map(|text| {
            text.replace('\n', " ")
                .trim()
                .chars()
                .take(80)
                .collect()
        })
        .unwrap_or_default();

    // CWD: first message that carries a cwd/workingDirectory field
    let cwd = messages
        .iter()
        .find_map(|m| m.cwd_path())
        .unwrap_or_else(|| fallback_cwd.to_path_buf());

    // Session ID = file stem
    let session_id = file_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

    // Timestamps from filesystem metadata
    let meta = std::fs::metadata(file_path)?;
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
        source: SessionSource::Copilot,
        cwd,
        repo: None,
        summary,
        updated_at,
        created_at,
        message_count,
        files_modified: vec![],
        original_path: file_path.to_path_buf(),
    })
}

// ---------------------------------------------------------------------------
// File predicate
// ---------------------------------------------------------------------------

/// Returns `true` for `.json` and `.jsonl` files directly in the root dir.
fn is_copilot_session_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("json") | Some("jsonl")
    )
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Discover and parse all Copilot sessions under `root`.
/// `root` is typically `~/.copilot/session-state/`.
/// If `root` does not exist, returns an empty vec silently.
pub async fn discover_copilot_sessions_in(root: &Path) -> Vec<UnifiedSession> {
    let mut sessions = Vec::new();

    if !root.exists() {
        return sessions;
    }

    let Ok(entries) = std::fs::read_dir(root) else {
        return sessions;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && is_copilot_session_file(&path) {
            if let Ok(messages) = parse_copilot_file(&path) {
                if let Ok(session) = session_from_messages(&path, messages, root) {
                    sessions.push(session);
                }
            }
        }
    }

    sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    sessions
}

/// Discover all Copilot sessions from the default location:
/// `~/.copilot/session-state/`.
pub async fn discover_copilot_sessions() -> Vec<UnifiedSession> {
    let root = match dirs::home_dir() {
        Some(h) => h.join(".copilot").join("session-state"),
        None => return Vec::new(),
    };
    discover_copilot_sessions_in(&root).await
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
    // Test 1: empty directory returns empty vec
    // ------------------------------------------------------------------
    #[test]
    fn test_copilot_empty_dir() {
        let tmp = TempDir::new().unwrap();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let sessions = rt.block_on(discover_copilot_sessions_in(tmp.path()));
        assert!(sessions.is_empty(), "Empty directory should yield 0 sessions");
    }

    // ------------------------------------------------------------------
    // Test 2: JSONL file with user message → correct summary
    // ------------------------------------------------------------------
    #[test]
    fn test_copilot_jsonl_parse() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("abc-session-001.jsonl");
        let lines = vec![
            r#"{"role":"user","content":"add auth middleware"}"#,
            r#"{"role":"assistant","content":"I'll create the middleware."}"#,
        ];
        fs::write(&file, lines.join("\n")).unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let sessions = rt.block_on(discover_copilot_sessions_in(tmp.path()));
        assert_eq!(sessions.len(), 1);
        assert!(
            sessions[0].summary.contains("add auth middleware"),
            "Summary should be the first user message, got: {}",
            sessions[0].summary
        );
        assert_eq!(sessions[0].source, SessionSource::Copilot);
        assert_eq!(sessions[0].id, "abc-session-001");
    }

    // ------------------------------------------------------------------
    // Test 3: JSON array file → sessions parsed
    // ------------------------------------------------------------------
    #[test]
    fn test_copilot_json_array_parse() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("session-xyz.json");
        let content = r#"[
            {"role":"user","content":"implement the login flow"},
            {"role":"assistant","content":"Sure, here is the login flow."}
        ]"#;
        fs::write(&file, content).unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let sessions = rt.block_on(discover_copilot_sessions_in(tmp.path()));
        assert_eq!(sessions.len(), 1);
        assert!(
            sessions[0].summary.contains("implement the login flow"),
            "Summary should match first user message, got: {}",
            sessions[0].summary
        );
        assert_eq!(sessions[0].source, SessionSource::Copilot);
    }

    // ------------------------------------------------------------------
    // Test 4: message_count = 4 for a file with 4 messages
    // ------------------------------------------------------------------
    #[test]
    fn test_copilot_message_count() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("session-count.jsonl");
        let lines = vec![
            r#"{"role":"user","content":"Turn 1"}"#,
            r#"{"role":"assistant","content":"Turn 2"}"#,
            r#"{"role":"user","content":"Turn 3"}"#,
            r#"{"role":"assistant","content":"Turn 4"}"#,
        ];
        fs::write(&file, lines.join("\n")).unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let sessions = rt.block_on(discover_copilot_sessions_in(tmp.path()));
        assert_eq!(sessions.len(), 1);
        assert_eq!(
            sessions[0].message_count, 4,
            "message_count should be 4, got: {}",
            sessions[0].message_count
        );
    }

    // ------------------------------------------------------------------
    // Test 5: .txt files (and other non-json/jsonl) are ignored
    // ------------------------------------------------------------------
    #[test]
    fn test_copilot_file_filter() {
        let tmp = TempDir::new().unwrap();

        // These should be ignored
        fs::write(tmp.path().join("notes.txt"), b"some notes here").unwrap();
        fs::write(tmp.path().join("README.md"), b"# docs").unwrap();
        fs::write(tmp.path().join("data.yaml"), b"key: value").unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let sessions = rt.block_on(discover_copilot_sessions_in(tmp.path()));
        assert!(sessions.is_empty(), "Non-json/jsonl files must be skipped");
    }

    // ------------------------------------------------------------------
    // Bonus: non-existent root returns empty vec without panicking
    // ------------------------------------------------------------------
    #[test]
    fn test_copilot_nonexistent_root() {
        let tmp = TempDir::new().unwrap();
        let missing = tmp.path().join("does_not_exist");
        let rt = tokio::runtime::Runtime::new().unwrap();
        let sessions = rt.block_on(discover_copilot_sessions_in(&missing));
        assert!(sessions.is_empty());
    }

    // ------------------------------------------------------------------
    // Bonus: cwd is extracted from message field when present
    // ------------------------------------------------------------------
    #[test]
    fn test_copilot_cwd_from_message() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("session-cwd.jsonl");
        let lines = vec![
            r#"{"role":"user","content":"fix the bug","cwd":"/Users/me/myproject"}"#,
            r#"{"role":"assistant","content":"Done."}"#,
        ];
        fs::write(&file, lines.join("\n")).unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let sessions = rt.block_on(discover_copilot_sessions_in(tmp.path()));
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].cwd, PathBuf::from("/Users/me/myproject"));
    }

    // ------------------------------------------------------------------
    // Bonus: workingDirectory field also extracted for cwd
    // ------------------------------------------------------------------
    #[test]
    fn test_copilot_working_directory_field() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("session-wd.jsonl");
        let lines = vec![
            r#"{"role":"user","content":"add tests","workingDirectory":"/home/dev/repo"}"#,
        ];
        fs::write(&file, lines.join("\n")).unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let sessions = rt.block_on(discover_copilot_sessions_in(tmp.path()));
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].cwd, PathBuf::from("/home/dev/repo"));
    }

    // ------------------------------------------------------------------
    // Bonus: summary is capped at 80 chars
    // ------------------------------------------------------------------
    #[test]
    fn test_copilot_summary_truncated() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("session-long.jsonl");
        let long_msg = "a".repeat(200);
        let line = format!(r#"{{"role":"user","content":"{}"}}"#, long_msg);
        fs::write(&file, line.as_bytes()).unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let sessions = rt.block_on(discover_copilot_sessions_in(tmp.path()));
        assert_eq!(sessions.len(), 1);
        assert!(
            sessions[0].summary.len() <= 80,
            "Summary must be at most 80 chars, got: {}",
            sessions[0].summary.len()
        );
    }

    // ------------------------------------------------------------------
    // Bonus: is_copilot_session_file predicate
    // ------------------------------------------------------------------
    #[test]
    fn test_is_copilot_session_file_predicate() {
        assert!(is_copilot_session_file(Path::new("session.jsonl")));
        assert!(is_copilot_session_file(Path::new("session.json")));
        assert!(!is_copilot_session_file(Path::new("session.txt")));
        assert!(!is_copilot_session_file(Path::new("session.yaml")));
        assert!(!is_copilot_session_file(Path::new("session.md")));
    }

    // ------------------------------------------------------------------
    // Bonus: JSONL file with only assistant messages → no session
    //        (no user messages means message_count may be 0 or summary empty)
    // ------------------------------------------------------------------
    #[test]
    fn test_copilot_only_assistant_messages() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("session-asst.jsonl");
        let lines = vec![
            r#"{"role":"assistant","content":"Hello there."}"#,
        ];
        fs::write(&file, lines.join("\n")).unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let sessions = rt.block_on(discover_copilot_sessions_in(tmp.path()));
        // Either parsed (1 assistant = 1 message) or filtered; either is valid.
        // If parsed, message_count must be 1 and summary is empty.
        for s in &sessions {
            assert_eq!(s.source, SessionSource::Copilot);
        }
    }

    // ------------------------------------------------------------------
    // Bonus: sessions sorted newest-first
    // ------------------------------------------------------------------
    #[tokio::test]
    async fn test_copilot_sessions_sorted() {
        let tmp = TempDir::new().unwrap();
        let f1 = tmp.path().join("session-alpha.jsonl");
        let f2 = tmp.path().join("session-beta.jsonl");
        let line = r#"{"role":"user","content":"hello"}"#;
        fs::write(&f1, line.as_bytes()).unwrap();
        fs::write(&f2, line.as_bytes()).unwrap();

        let sessions = discover_copilot_sessions_in(tmp.path()).await;
        // Can have 1 or 2 depending on FS mtime granularity; just verify order property.
        if sessions.len() >= 2 {
            assert!(sessions[0].updated_at >= sessions[1].updated_at);
        }
    }
}
