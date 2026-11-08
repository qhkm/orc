use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::io::BufRead;
use std::path::Path;
use crate::session::types::{SessionSource, UnifiedSession};

// ---------------------------------------------------------------------------
// Serde structs
//
// Amp stores sessions under ~/.amp/sessions/ or ~/.config/amp/sessions/.
// This is a best-guess stub implementation — the exact format is TBD.
//
// Supported message formats (`.json` or `.jsonl` files):
//   { "role": "user",      "content": "..." }
//   { "role": "assistant", "content": "..." }
//
// Session ID: filename stem.
// CWD: falls back to the root discovery directory (no CWD in Amp format TBD).
// Summary: first user message (up to 80 chars).
// ---------------------------------------------------------------------------

/// A single message parsed from an Amp session file.
#[derive(Debug, Deserialize)]
struct AmpMessage {
    role: Option<String>,
    content: Option<String>,
}

impl AmpMessage {
    fn is_user(&self) -> bool {
        self.role.as_deref() == Some("user")
    }

    fn is_countable(&self) -> bool {
        matches!(self.role.as_deref(), Some("user") | Some("assistant"))
    }
}

// ---------------------------------------------------------------------------
// File parsing helpers
// ---------------------------------------------------------------------------

/// Parse a JSONL file (one JSON object per non-empty line).
fn parse_jsonl(file_path: &Path) -> Result<Vec<AmpMessage>> {
    let file = std::fs::File::open(file_path)?;
    let reader = std::io::BufReader::new(file);
    let mut messages = Vec::new();
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(msg) = serde_json::from_str::<AmpMessage>(&line) {
            messages.push(msg);
        }
    }
    Ok(messages)
}

/// Parse a JSON file containing an array of message objects.
fn parse_json_array(file_path: &Path) -> Result<Vec<AmpMessage>> {
    let content = std::fs::read_to_string(file_path)?;
    let messages: Vec<AmpMessage> = serde_json::from_str(&content)?;
    Ok(messages)
}

/// Parse an Amp session file (either `.json` array or `.jsonl`).
fn parse_amp_file(file_path: &Path) -> Result<Vec<AmpMessage>> {
    let ext = file_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    match ext {
        "jsonl" => parse_jsonl(file_path),
        "json" => {
            // Try JSON array first; fall back to JSONL line-by-line
            parse_json_array(file_path).or_else(|_| parse_jsonl(file_path))
        }
        _ => Err(anyhow::anyhow!("unsupported extension: {:?}", file_path)),
    }
}

// ---------------------------------------------------------------------------
// Single file → UnifiedSession
// ---------------------------------------------------------------------------

fn session_from_messages(
    file_path: &Path,
    messages: Vec<AmpMessage>,
    fallback_cwd: &Path,
) -> Result<UnifiedSession> {
    if messages.is_empty() {
        return Err(anyhow::anyhow!("empty session: {:?}", file_path));
    }

    let message_count = messages.iter().filter(|m| m.is_countable()).count();
    if message_count == 0 {
        return Err(anyhow::anyhow!(
            "no user/assistant messages: {:?}",
            file_path
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
        source: SessionSource::Amp,
        cwd: fallback_cwd.to_path_buf(),
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

/// Returns `true` for `.json` and `.jsonl` files.
fn is_amp_session_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("json") | Some("jsonl")
    )
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Discover and parse all Amp sessions under `root`.
/// `root` is typically `~/.amp/sessions/` or `~/.config/amp/sessions/`.
/// If `root` does not exist, returns an empty vec silently.
pub async fn discover_amp_sessions_in(root: &Path) -> Vec<UnifiedSession> {
    let mut sessions = Vec::new();

    if !root.exists() {
        return sessions;
    }

    let Ok(entries) = std::fs::read_dir(root) else {
        return sessions;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && is_amp_session_file(&path) {
            if let Ok(messages) = parse_amp_file(&path) {
                if let Ok(session) = session_from_messages(&path, messages, root) {
                    sessions.push(session);
                }
            }
        }
    }

    sessions.sort_by_key(|s| std::cmp::Reverse(s.updated_at)); // newest first
    sessions
}

/// Discover all Amp sessions from the default locations:
///   1. `~/.amp/sessions/`
///   2. `~/.config/amp/sessions/` (fallback if the first doesn't exist)
pub async fn discover_amp_sessions() -> Vec<UnifiedSession> {
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => return Vec::new(),
    };

    // Prefer ~/.amp/sessions; fall back to ~/.config/amp/sessions
    let primary = home.join(".amp").join("sessions");
    if primary.exists() {
        return discover_amp_sessions_in(&primary).await;
    }

    let secondary = home.join(".config").join("amp").join("sessions");
    discover_amp_sessions_in(&secondary).await
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
    // Test 1: missing/empty root → empty vec
    // ------------------------------------------------------------------
    #[test]
    fn test_amp_empty_dir() {
        let tmp = TempDir::new().unwrap();
        let missing = tmp.path().join("does_not_exist");
        let rt = tokio::runtime::Runtime::new().unwrap();
        let sessions = rt.block_on(discover_amp_sessions_in(&missing));
        assert!(sessions.is_empty(), "Missing root should yield 0 sessions");
    }

    // ------------------------------------------------------------------
    // Test 2: basic parse — .jsonl file with user message → correct session
    // ------------------------------------------------------------------
    #[test]
    fn test_amp_basic_parse_jsonl() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("session-amp-001.jsonl");
        let lines = [r#"{"role":"user","content":"refactor the auth service"}"#,
            r#"{"role":"assistant","content":"I'll refactor it."}"#];
        fs::write(&file, lines.join("\n")).unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let sessions = rt.block_on(discover_amp_sessions_in(tmp.path()));
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].source, SessionSource::Amp);
        assert_eq!(sessions[0].id, "session-amp-001");
        assert!(
            sessions[0].summary.contains("refactor the auth service"),
            "Summary should be the first user message, got: {}",
            sessions[0].summary
        );
    }

    // ------------------------------------------------------------------
    // Test 3: message_count — N messages → correct count
    // ------------------------------------------------------------------
    #[test]
    fn test_amp_message_count() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("session-count.jsonl");
        let lines = [r#"{"role":"user","content":"Turn 1"}"#,
            r#"{"role":"assistant","content":"Turn 2"}"#,
            r#"{"role":"user","content":"Turn 3"}"#,
            r#"{"role":"assistant","content":"Turn 4"}"#,
            r#"{"role":"user","content":"Turn 5"}"#];
        fs::write(&file, lines.join("\n")).unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let sessions = rt.block_on(discover_amp_sessions_in(tmp.path()));
        assert_eq!(sessions.len(), 1);
        assert_eq!(
            sessions[0].message_count, 5,
            "message_count should be 5, got: {}",
            sessions[0].message_count
        );
    }

    // ------------------------------------------------------------------
    // Test 4: basic parse — .json array file → correct session
    // ------------------------------------------------------------------
    #[test]
    fn test_amp_basic_parse_json_array() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("session-json.json");
        let content = r#"[
            {"role":"user","content":"write unit tests for parser"},
            {"role":"assistant","content":"Sure, writing tests now."}
        ]"#;
        fs::write(&file, content).unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let sessions = rt.block_on(discover_amp_sessions_in(tmp.path()));
        assert_eq!(sessions.len(), 1);
        assert!(
            sessions[0].summary.contains("write unit tests for parser"),
            "JSON array format should parse correctly, got: {}",
            sessions[0].summary
        );
        assert_eq!(sessions[0].message_count, 2);
    }

    // ------------------------------------------------------------------
    // Test 5: non-.json/.jsonl files are ignored
    // ------------------------------------------------------------------
    #[test]
    fn test_amp_file_filter() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("notes.txt"), b"text").unwrap();
        fs::write(tmp.path().join("config.yaml"), b"key: value").unwrap();
        fs::write(tmp.path().join("README.md"), b"# readme").unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let sessions = rt.block_on(discover_amp_sessions_in(tmp.path()));
        assert!(sessions.is_empty(), "Non-json/jsonl files should be ignored");
    }

    // ------------------------------------------------------------------
    // Test 6: empty root directory → empty vec
    // ------------------------------------------------------------------
    #[test]
    fn test_amp_root_exists_but_empty() {
        let tmp = TempDir::new().unwrap();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let sessions = rt.block_on(discover_amp_sessions_in(tmp.path()));
        assert!(sessions.is_empty(), "Empty root directory should yield 0 sessions");
    }

    // ------------------------------------------------------------------
    // Test 7: summary is capped at 80 chars
    // ------------------------------------------------------------------
    #[test]
    fn test_amp_summary_truncated() {
        let tmp = TempDir::new().unwrap();
        let long_msg = "a".repeat(200);
        let line = format!(r#"{{"role":"user","content":"{}"}}"#, long_msg);
        let file = tmp.path().join("session-long.jsonl");
        fs::write(&file, line.as_bytes()).unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let sessions = rt.block_on(discover_amp_sessions_in(tmp.path()));
        assert_eq!(sessions.len(), 1);
        assert!(
            sessions[0].summary.len() <= 80,
            "Summary must be at most 80 chars, got: {}",
            sessions[0].summary.len()
        );
    }

    // ------------------------------------------------------------------
    // Test 8: is_amp_session_file predicate
    // ------------------------------------------------------------------
    #[test]
    fn test_amp_session_file_predicate() {
        assert!(is_amp_session_file(Path::new("session.jsonl")));
        assert!(is_amp_session_file(Path::new("session.json")));
        assert!(!is_amp_session_file(Path::new("session.txt")));
        assert!(!is_amp_session_file(Path::new("session.yaml")));
        assert!(!is_amp_session_file(Path::new("session.md")));
    }

    // ------------------------------------------------------------------
    // Bonus: sessions sorted newest-first
    // ------------------------------------------------------------------
    #[tokio::test]
    async fn test_amp_sessions_sorted() {
        let tmp = TempDir::new().unwrap();
        let line = r#"{"role":"user","content":"hello"}"#;
        fs::write(tmp.path().join("session-alpha.jsonl"), line.as_bytes()).unwrap();
        fs::write(tmp.path().join("session-beta.jsonl"), line.as_bytes()).unwrap();

        let sessions = discover_amp_sessions_in(tmp.path()).await;
        if sessions.len() >= 2 {
            assert!(sessions[0].updated_at >= sessions[1].updated_at);
        }
    }
}
