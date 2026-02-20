use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::io::BufRead;
use std::path::Path;
use crate::session::types::{SessionSource, UnifiedSession};

// ---------------------------------------------------------------------------
// Serde structs
//
// Kimi CLI stores sessions under ~/.kimi/sessions/.
// Sessions may be stored as:
//   <session-id>.jsonl  — JSONL file, one message per line.
//   <session-id>/history.jsonl — directory with JSONL inside.
//
// Supported message formats:
//   { "role": "user",    "content": "..." }
//   { "role": "model",   "content": "..." }
//   { "role": "assistant","content": "..." }
//
// Session ID: filename stem (for flat files) or directory name.
// CWD: falls back to the root discovery directory (no CWD in Kimi format).
// Summary: first user message (up to 80 chars).
// ---------------------------------------------------------------------------

/// A single message parsed from a Kimi session file.
#[derive(Debug, Deserialize)]
struct KimiMessage {
    role: Option<String>,
    content: Option<String>,
}

impl KimiMessage {
    fn is_user(&self) -> bool {
        self.role.as_deref() == Some("user")
    }

    fn is_countable(&self) -> bool {
        matches!(
            self.role.as_deref(),
            Some("user") | Some("model") | Some("assistant")
        )
    }
}

// ---------------------------------------------------------------------------
// JSONL parsing
// ---------------------------------------------------------------------------

/// Parse a JSONL file into a vec of `KimiMessage`s.
/// Bad lines are silently skipped.
fn parse_jsonl(file_path: &Path) -> Result<Vec<KimiMessage>> {
    let file = std::fs::File::open(file_path)?;
    let reader = std::io::BufReader::new(file);
    let mut messages = Vec::new();
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(msg) = serde_json::from_str::<KimiMessage>(&line) {
            messages.push(msg);
        }
    }
    Ok(messages)
}

// ---------------------------------------------------------------------------
// Single file/directory → UnifiedSession
// ---------------------------------------------------------------------------

fn session_from_messages(
    session_path: &Path,   // the path used for metadata (file or dir)
    history_path: &Path,   // actual file to read mtime from
    session_id: &str,
    messages: Vec<KimiMessage>,
    fallback_cwd: &Path,
) -> Result<UnifiedSession> {
    if messages.is_empty() {
        return Err(anyhow::anyhow!("empty session: {:?}", session_path));
    }

    let message_count = messages.iter().filter(|m| m.is_countable()).count();
    if message_count == 0 {
        return Err(anyhow::anyhow!(
            "no user/model/assistant messages: {:?}",
            session_path
        ));
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

    // Timestamps from filesystem metadata of the actual file
    let meta = std::fs::metadata(history_path)?;
    let updated_at = meta
        .modified()
        .map(DateTime::<Utc>::from)
        .unwrap_or_else(|_| Utc::now());
    let created_at = meta
        .created()
        .map(DateTime::<Utc>::from)
        .unwrap_or(updated_at);

    Ok(UnifiedSession {
        id: session_id.to_string(),
        source: SessionSource::Kimi,
        cwd: fallback_cwd.to_path_buf(),
        repo: None,
        summary,
        updated_at,
        created_at,
        message_count,
        files_modified: vec![],
        original_path: history_path.to_path_buf(),
    })
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

/// Returns `true` for `.jsonl` files directly in the sessions root.
fn is_kimi_flat_file(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()) == Some("jsonl")
}

/// Discover and parse all Kimi sessions under `root`.
/// `root` is typically `~/.kimi/sessions/`.
/// Handles two layouts:
///   Flat: `<root>/<id>.jsonl`
///   Directory: `<root>/<id>/history.jsonl`
/// If `root` does not exist, returns an empty vec silently.
pub async fn discover_kimi_sessions_in(root: &Path) -> Vec<UnifiedSession> {
    let mut sessions = Vec::new();

    if !root.exists() {
        return sessions;
    }

    let Ok(entries) = std::fs::read_dir(root) else {
        return sessions;
    };

    for entry in entries.flatten() {
        let path = entry.path();

        if path.is_file() && is_kimi_flat_file(&path) {
            // Flat JSONL file layout
            let session_id = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown");
            if let Ok(messages) = parse_jsonl(&path) {
                if let Ok(session) =
                    session_from_messages(&path, &path, session_id, messages, root)
                {
                    sessions.push(session);
                }
            }
        } else if path.is_dir() {
            // Directory layout: look for history.jsonl inside
            let history = path.join("history.jsonl");
            if history.exists() {
                let session_id = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown");
                if let Ok(messages) = parse_jsonl(&history) {
                    if let Ok(session) =
                        session_from_messages(&path, &history, session_id, messages, root)
                    {
                        sessions.push(session);
                    }
                }
            }
        }
    }

    sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    sessions
}

/// Discover all Kimi sessions from the default location: `~/.kimi/sessions/`.
pub async fn discover_kimi_sessions() -> Vec<UnifiedSession> {
    let root = match dirs::home_dir() {
        Some(h) => h.join(".kimi").join("sessions"),
        None => return Vec::new(),
    };
    discover_kimi_sessions_in(&root).await
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
    // Test 1: missing root → empty vec
    // ------------------------------------------------------------------
    #[test]
    fn test_kimi_empty_dir() {
        let tmp = TempDir::new().unwrap();
        let missing = tmp.path().join("does_not_exist");
        let rt = tokio::runtime::Runtime::new().unwrap();
        let sessions = rt.block_on(discover_kimi_sessions_in(&missing));
        assert!(sessions.is_empty(), "Missing root should yield 0 sessions");
    }

    // ------------------------------------------------------------------
    // Test 2: basic parse — flat .jsonl file with user message
    // ------------------------------------------------------------------
    #[test]
    fn test_kimi_basic_parse_flat() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("session-abc.jsonl");
        let lines = vec![
            r#"{"role":"user","content":"implement the login flow"}"#,
            r#"{"role":"assistant","content":"Sure, here is the login flow."}"#,
        ];
        fs::write(&file, lines.join("\n")).unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let sessions = rt.block_on(discover_kimi_sessions_in(tmp.path()));
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].source, SessionSource::Kimi);
        assert_eq!(sessions[0].id, "session-abc");
        assert!(
            sessions[0].summary.contains("implement the login flow"),
            "Summary should be the first user message, got: {}",
            sessions[0].summary
        );
    }

    // ------------------------------------------------------------------
    // Test 3: message_count — N messages → correct count (flat file)
    // ------------------------------------------------------------------
    #[test]
    fn test_kimi_message_count_flat() {
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
        let sessions = rt.block_on(discover_kimi_sessions_in(tmp.path()));
        assert_eq!(sessions.len(), 1);
        assert_eq!(
            sessions[0].message_count, 4,
            "message_count should be 4, got: {}",
            sessions[0].message_count
        );
    }

    // ------------------------------------------------------------------
    // Test 4: basic parse — directory layout with history.jsonl
    // ------------------------------------------------------------------
    #[test]
    fn test_kimi_basic_parse_directory() {
        let tmp = TempDir::new().unwrap();
        let session_dir = tmp.path().join("session-dir-001");
        fs::create_dir_all(&session_dir).unwrap();
        let lines = vec![
            r#"{"role":"user","content":"add rate limiting"}"#,
            r#"{"role":"model","content":"I'll add rate limiting."}"#,
        ];
        fs::write(session_dir.join("history.jsonl"), lines.join("\n")).unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let sessions = rt.block_on(discover_kimi_sessions_in(tmp.path()));
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].source, SessionSource::Kimi);
        assert_eq!(sessions[0].id, "session-dir-001");
        assert!(
            sessions[0].summary.contains("add rate limiting"),
            "Directory-layout summary should work, got: {}",
            sessions[0].summary
        );
    }

    // ------------------------------------------------------------------
    // Test 5: message_count — directory layout
    // ------------------------------------------------------------------
    #[test]
    fn test_kimi_message_count_directory() {
        let tmp = TempDir::new().unwrap();
        let session_dir = tmp.path().join("session-dir-count");
        fs::create_dir_all(&session_dir).unwrap();
        let lines = vec![
            r#"{"role":"user","content":"Q1"}"#,
            r#"{"role":"model","content":"A1"}"#,
            r#"{"role":"user","content":"Q2"}"#,
        ];
        fs::write(session_dir.join("history.jsonl"), lines.join("\n")).unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let sessions = rt.block_on(discover_kimi_sessions_in(tmp.path()));
        assert_eq!(sessions.len(), 1);
        assert_eq!(
            sessions[0].message_count, 3,
            "message_count should be 3, got: {}",
            sessions[0].message_count
        );
    }

    // ------------------------------------------------------------------
    // Test 6: "model" role counts as a message
    // ------------------------------------------------------------------
    #[test]
    fn test_kimi_model_role_counted() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("session-model.jsonl");
        let lines = vec![
            r#"{"role":"user","content":"ask kimi something"}"#,
            r#"{"role":"model","content":"kimi reply"}"#,
        ];
        fs::write(&file, lines.join("\n")).unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let sessions = rt.block_on(discover_kimi_sessions_in(tmp.path()));
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].message_count, 2);
    }

    // ------------------------------------------------------------------
    // Test 7: non-.jsonl files in root are ignored
    // ------------------------------------------------------------------
    #[test]
    fn test_kimi_file_filter() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("notes.txt"), b"text").unwrap();
        fs::write(tmp.path().join("data.json"), b"{}").unwrap();
        fs::write(tmp.path().join("log.md"), b"# log").unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let sessions = rt.block_on(discover_kimi_sessions_in(tmp.path()));
        assert!(sessions.is_empty(), "Non-.jsonl files should be ignored");
    }

    // ------------------------------------------------------------------
    // Test 8: directories without history.jsonl are skipped
    // ------------------------------------------------------------------
    #[test]
    fn test_kimi_dir_without_history_skipped() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("no-history-dir")).unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let sessions = rt.block_on(discover_kimi_sessions_in(tmp.path()));
        assert!(sessions.is_empty(), "Dirs without history.jsonl should be skipped");
    }

    // ------------------------------------------------------------------
    // Test 9: summary is capped at 80 chars
    // ------------------------------------------------------------------
    #[test]
    fn test_kimi_summary_truncated() {
        let tmp = TempDir::new().unwrap();
        let long_msg = "k".repeat(200);
        let line = format!(r#"{{"role":"user","content":"{}"}}"#, long_msg);
        let file = tmp.path().join("session-long.jsonl");
        fs::write(&file, line.as_bytes()).unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let sessions = rt.block_on(discover_kimi_sessions_in(tmp.path()));
        assert_eq!(sessions.len(), 1);
        assert!(
            sessions[0].summary.len() <= 80,
            "Summary must be at most 80 chars, got: {}",
            sessions[0].summary.len()
        );
    }

    // ------------------------------------------------------------------
    // Bonus: sessions sorted newest-first
    // ------------------------------------------------------------------
    #[tokio::test]
    async fn test_kimi_sessions_sorted() {
        let tmp = TempDir::new().unwrap();
        let line = r#"{"role":"user","content":"hello"}"#;
        fs::write(tmp.path().join("session-alpha.jsonl"), line.as_bytes()).unwrap();
        fs::write(tmp.path().join("session-beta.jsonl"), line.as_bytes()).unwrap();

        let sessions = discover_kimi_sessions_in(tmp.path()).await;
        if sessions.len() >= 2 {
            assert!(sessions[0].updated_at >= sessions[1].updated_at);
        }
    }
}
