use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use crate::session::types::{SessionSource, UnifiedSession};

// ---------------------------------------------------------------------------
// Serde structs — handles two observed formats:
//
// Format A (actual Gemini CLI logs.json):
//   [{ "sessionId": "...", "messageId": 0, "type": "user", "message": "...", "timestamp": "..." }]
//
// Format B (described in task / Gemini API style):
//   [{ "role": "user", "parts": [{ "text": "..." }] }]
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct GeminiPart {
    text: Option<String>,
}

/// A single turn in a Gemini session file.
/// Both formats are handled via optional fields.
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct GeminiTurn {
    // Format A fields
    #[serde(rename = "type")]
    turn_type: Option<String>,
    message: Option<String>,
    timestamp: Option<String>,
    #[serde(rename = "sessionId")]
    session_id: Option<String>,

    // Format B fields
    role: Option<String>,
    parts: Option<Vec<GeminiPart>>,
}

impl GeminiTurn {
    /// Return the role/type string in a normalised form: "user" or "model".
    fn role_str(&self) -> &str {
        if let Some(t) = &self.turn_type {
            return t.as_str();
        }
        if let Some(r) = &self.role {
            return r.as_str();
        }
        ""
    }

    fn is_user(&self) -> bool {
        self.role_str() == "user"
    }

    fn is_message(&self) -> bool {
        matches!(self.role_str(), "user" | "model")
    }

    /// Extract text content from this turn.
    fn text(&self) -> String {
        // Format A: plain `message` string
        if let Some(m) = &self.message {
            if !m.is_empty() {
                return m.clone();
            }
        }
        // Format B: parts array
        if let Some(parts) = &self.parts {
            let joined: String = parts
                .iter()
                .filter_map(|p| p.text.as_deref())
                .collect::<Vec<_>>()
                .join("\n");
            if !joined.is_empty() {
                return joined;
            }
        }
        String::new()
    }
}

// ---------------------------------------------------------------------------
// CWD derivation helpers
// ---------------------------------------------------------------------------

/// Try to decode a Gemini session-dir name as a URL-encoded path.
/// Gemini CLI hashes project directories (SHA-256 hex), so this almost
/// always fails — we fall back to `~/.gemini/tmp/<session_dir>` in that case.
fn decode_cwd(session_dir: &str, gemini_tmp: &Path) -> PathBuf {
    // URL-percent-decode (e.g. "%2Fhome%2Fuser%2Fproject" → "/home/user/project")
    let decoded = percent_decode(session_dir);
    // Heuristic: a valid path starts with '/' (absolute) and has no spaces.
    if decoded.starts_with('/') && !decoded.contains(' ') {
        return PathBuf::from(decoded);
    }
    // Fallback
    gemini_tmp.join(session_dir)
}

/// Very small percent-decoder (handles %XX sequences only).
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// File parsing
// ---------------------------------------------------------------------------

/// Parse a single Gemini `logs.json` (or any `.json` in a `chats/` dir)
/// into a `UnifiedSession`.  Returns `Err` if the file is unusable.
fn parse_gemini_file(file_path: &Path, gemini_tmp: &Path) -> Result<UnifiedSession> {
    // Enforce minimum size (50 bytes) to skip empty/stub files
    let meta = std::fs::metadata(file_path)?;
    if meta.len() < 50 {
        return Err(anyhow::anyhow!("file too small: {:?}", file_path));
    }

    let content = std::fs::read_to_string(file_path)?;
    let turns: Vec<GeminiTurn> = serde_json::from_str(&content)?;

    if turns.is_empty() {
        return Err(anyhow::anyhow!("empty session: {:?}", file_path));
    }

    // Count messages (user + model turns only)
    let message_count = turns.iter().filter(|t| t.is_message()).count();
    if message_count == 0 {
        return Err(anyhow::anyhow!("no user/model messages: {:?}", file_path));
    }

    // Summary = first user message text (truncated to 80 chars)
    let summary: String = turns
        .iter()
        .find(|t| t.is_user())
        .map(|t| {
            t.text()
                .replace('\n', " ")
                .trim()
                .chars()
                .take(80)
                .collect()
        })
        .unwrap_or_default();

    // Session ID: prefer the `sessionId` field from the first turn that has it;
    // fall back to the session-dir folder name (two levels up from `chats/<file>`
    // or one level up for `logs.json` directly under the session dir).
    let session_id_from_turn = turns.iter().find_map(|t| t.session_id.clone());

    // Derive session_dir name from file path
    let session_dir_name = derive_session_dir(file_path);
    let session_id = session_id_from_turn.unwrap_or_else(|| session_dir_name.clone());

    // CWD
    let cwd = decode_cwd(&session_dir_name, gemini_tmp);

    // Timestamps from filesystem metadata
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
        source: SessionSource::Gemini,
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

/// Determine the session-dir name from a file path.
///
/// Handles two layouts:
/// - `~/.gemini/tmp/<session-dir>/chats/<file>.json`  → parent of parent
/// - `~/.gemini/tmp/<session-dir>/logs.json`           → parent
fn derive_session_dir(file_path: &Path) -> String {
    let parent = file_path.parent();
    // If the immediate parent is `chats`, go one level higher
    if parent
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        == Some("chats")
    {
        parent
            .and_then(|p| p.parent())
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string()
    } else {
        parent
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string()
    }
}

// ---------------------------------------------------------------------------
// Discovery — file predicate
// ---------------------------------------------------------------------------

/// Returns `true` for `.json` files inside a `chats/` subdirectory,
/// OR for `logs.json` files directly under a session-dir.
fn is_gemini_session_file(path: &Path) -> bool {
    if !path.extension().and_then(|e| e.to_str()).is_some_and(|e| e == "json") {
        return false;
    }
    let parent_name = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("");

    // Files inside a `chats/` directory
    if parent_name == "chats" {
        return true;
    }

    // `logs.json` directly under a session-dir (actual Gemini CLI layout)
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        if name == "logs.json" {
            return true;
        }
    }

    false
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Discover and parse all Gemini sessions under `root`.
/// `root` is typically `~/.gemini/tmp/`.
pub async fn discover_gemini_sessions_in(root: &Path) -> Vec<UnifiedSession> {
    let mut sessions = Vec::new();

    if !root.exists() {
        return sessions;
    }

    // Iterative DFS walk
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.is_file() && is_gemini_session_file(&path) {
                if let Ok(session) = parse_gemini_file(&path, root) {
                    sessions.push(session);
                }
            }
        }
    }

    sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    sessions
}

/// Discover all Gemini sessions from the default location: `~/.gemini/tmp/`.
pub async fn discover_gemini_sessions() -> Vec<UnifiedSession> {
    let root = match dirs::home_dir() {
        Some(h) => h.join(".gemini").join("tmp"),
        None => return Vec::new(),
    };
    discover_gemini_sessions_in(&root).await
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
    // Helpers
    // ------------------------------------------------------------------

    /// Write a `logs.json` fixture directly under `session_dir` using
    /// Format A (the actual Gemini CLI layout).
    fn write_format_a_fixture(session_dir: &Path, messages: &[(&str, &str)]) {
        fs::create_dir_all(session_dir).unwrap();
        let entries: Vec<String> = messages
            .iter()
            .enumerate()
            .map(|(i, (role, text))| {
                format!(
                    r#"{{"sessionId":"sess-001","messageId":{i},"type":"{role}","message":"{text}","timestamp":"2026-02-20T10:0{i}:00Z"}}"#,
                    i = i, role = role, text = text
                )
            })
            .collect();
        let content = format!("[{}]", entries.join(",\n"));
        fs::write(session_dir.join("logs.json"), content).unwrap();
    }

    /// Write a `chat.json` fixture inside a `chats/` subdirectory using
    /// Format B (role + parts / Gemini API style).
    fn write_format_b_fixture(session_dir: &Path, messages: &[(&str, &str)]) {
        let chats_dir = session_dir.join("chats");
        fs::create_dir_all(&chats_dir).unwrap();
        let entries: Vec<String> = messages
            .iter()
            .map(|(role, text)| {
                format!(
                    r#"{{"role":"{role}","parts":[{{"text":"{text}"}}]}}"#,
                    role = role, text = text
                )
            })
            .collect();
        let content = format!("[{}]", entries.join(",\n"));
        fs::write(chats_dir.join("chat.json"), content).unwrap();
    }

    // ------------------------------------------------------------------
    // Test 1: empty JSON array → 0 sessions
    // ------------------------------------------------------------------
    #[test]
    fn test_parse_gemini_empty_array() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().to_path_buf();
        let session_dir = root.join("session-empty");
        fs::create_dir_all(&session_dir).unwrap();
        // Write an empty array but with enough bytes for the size check
        fs::write(
            session_dir.join("logs.json"),
            b"[                                                          ]",
        )
        .unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let sessions = rt.block_on(discover_gemini_sessions_in(&root));
        assert!(sessions.is_empty(), "Empty array should yield 0 sessions");
    }

    // ------------------------------------------------------------------
    // Test 2: single user message → correct summary (Format A)
    // ------------------------------------------------------------------
    #[test]
    fn test_parse_gemini_single_turn() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().to_path_buf();
        let session_dir = root.join("session-abc");
        write_format_a_fixture(&session_dir, &[("user", "Hello from Gemini")]);

        let rt = tokio::runtime::Runtime::new().unwrap();
        let sessions = rt.block_on(discover_gemini_sessions_in(&root));
        assert_eq!(sessions.len(), 1);
        assert!(
            sessions[0].summary.contains("Hello from Gemini"),
            "Summary should be the first user message, got: {}",
            sessions[0].summary
        );
        assert_eq!(sessions[0].source, SessionSource::Gemini);
    }

    // ------------------------------------------------------------------
    // Test 3: model turns are counted but not used as summary
    // ------------------------------------------------------------------
    #[test]
    fn test_parse_gemini_model_role() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().to_path_buf();
        let session_dir = root.join("session-model");
        write_format_a_fixture(
            &session_dir,
            &[
                ("model", "I am a model response"),
                ("user", "First real user message"),
            ],
        );

        let rt = tokio::runtime::Runtime::new().unwrap();
        let sessions = rt.block_on(discover_gemini_sessions_in(&root));
        assert_eq!(sessions.len(), 1);
        // Summary must come from the user turn, not the model turn
        assert!(
            sessions[0].summary.contains("First real user message"),
            "Summary should skip model turns; got: {}",
            sessions[0].summary
        );
    }

    // ------------------------------------------------------------------
    // Test 4: non-JSON files are skipped
    // ------------------------------------------------------------------
    #[test]
    fn test_gemini_file_filter() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().to_path_buf();
        let session_dir = root.join("session-filter");
        fs::create_dir_all(&session_dir).unwrap();

        // These should be ignored
        fs::write(session_dir.join("notes.txt"), b"some text content here").unwrap();
        fs::write(session_dir.join("data.jsonl"), b"not json array").unwrap();
        // Even a `.json` not named `logs.json` and not in `chats/` is skipped
        fs::write(session_dir.join("other.json"), b"{}").unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let sessions = rt.block_on(discover_gemini_sessions_in(&root));
        assert!(sessions.is_empty(), "Non-matching files must be skipped");
    }

    // ------------------------------------------------------------------
    // Test 5: message_count counts all user + model turns
    // ------------------------------------------------------------------
    #[test]
    fn test_gemini_message_count() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().to_path_buf();
        let session_dir = root.join("session-count");
        write_format_a_fixture(
            &session_dir,
            &[
                ("user", "Turn 1"),
                ("model", "Turn 2"),
                ("user", "Turn 3"),
            ],
        );

        let rt = tokio::runtime::Runtime::new().unwrap();
        let sessions = rt.block_on(discover_gemini_sessions_in(&root));
        assert_eq!(sessions.len(), 1);
        assert_eq!(
            sessions[0].message_count, 3,
            "message_count should be 3 (2 user + 1 model)"
        );
    }

    // ------------------------------------------------------------------
    // Test 6: empty directory returns empty vec
    // ------------------------------------------------------------------
    #[test]
    fn test_discover_gemini_sessions_in_empty_dir() {
        let tmp = TempDir::new().unwrap();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let sessions = rt.block_on(discover_gemini_sessions_in(tmp.path()));
        assert!(sessions.is_empty(), "Empty directory should yield 0 sessions");
    }

    // ------------------------------------------------------------------
    // Bonus: Format B (role + parts style) works too
    // ------------------------------------------------------------------
    #[test]
    fn test_parse_gemini_format_b_parts() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().to_path_buf();
        let session_dir = root.join("session-parts");
        write_format_b_fixture(
            &session_dir,
            &[
                ("user", "Parts-style user message"),
                ("model", "Parts-style model reply"),
            ],
        );

        let rt = tokio::runtime::Runtime::new().unwrap();
        let sessions = rt.block_on(discover_gemini_sessions_in(&root));
        assert_eq!(sessions.len(), 1);
        assert!(
            sessions[0].summary.contains("Parts-style user message"),
            "Format B parts should be parsed correctly; got: {}",
            sessions[0].summary
        );
        assert_eq!(sessions[0].message_count, 2);
    }

    // ------------------------------------------------------------------
    // Bonus: non-existent root returns empty vec without panicking
    // ------------------------------------------------------------------
    #[test]
    fn test_nonexistent_root_returns_empty() {
        let tmp = TempDir::new().unwrap();
        let missing = tmp.path().join("does_not_exist");
        let rt = tokio::runtime::Runtime::new().unwrap();
        let sessions = rt.block_on(discover_gemini_sessions_in(&missing));
        assert!(sessions.is_empty());
    }

    // ------------------------------------------------------------------
    // Bonus: session_id comes from turn's sessionId field when present
    // ------------------------------------------------------------------
    #[test]
    fn test_session_id_from_turn_field() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().to_path_buf();
        let session_dir = root.join("session-id-test");
        write_format_a_fixture(&session_dir, &[("user", "some question")]);

        let rt = tokio::runtime::Runtime::new().unwrap();
        let sessions = rt.block_on(discover_gemini_sessions_in(&root));
        assert_eq!(sessions.len(), 1);
        // The fixture embeds sessionId "sess-001"
        assert_eq!(sessions[0].id, "sess-001");
    }

    // ------------------------------------------------------------------
    // Bonus: small files (< 50 bytes) are filtered out
    // ------------------------------------------------------------------
    #[test]
    fn test_small_file_filtered() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().to_path_buf();
        let session_dir = root.join("session-small");
        fs::create_dir_all(&session_dir).unwrap();
        fs::write(session_dir.join("logs.json"), b"[]").unwrap(); // only 2 bytes

        let rt = tokio::runtime::Runtime::new().unwrap();
        let sessions = rt.block_on(discover_gemini_sessions_in(&root));
        assert!(sessions.is_empty(), "Files under 50 bytes should be filtered out");
    }

    // ------------------------------------------------------------------
    // Bonus: is_gemini_session_file predicate
    // ------------------------------------------------------------------
    #[test]
    fn test_is_gemini_session_file_predicate() {
        // logs.json directly under session-dir: parent has arbitrary name
        assert!(is_gemini_session_file(Path::new(
            "/home/user/.gemini/tmp/abc123/logs.json"
        )));
        // .json inside chats/ dir
        assert!(is_gemini_session_file(Path::new(
            "/home/user/.gemini/tmp/abc123/chats/session-1.json"
        )));
        // Non-JSON files are rejected
        assert!(!is_gemini_session_file(Path::new("notes.txt")));
        assert!(!is_gemini_session_file(Path::new("session.jsonl")));
        // A .json not named logs.json and not in chats/ is rejected
        assert!(!is_gemini_session_file(Path::new(
            "/home/user/.gemini/tmp/abc123/other.json"
        )));
    }

    // ------------------------------------------------------------------
    // Bonus: percent_decode helper
    // ------------------------------------------------------------------
    #[test]
    fn test_percent_decode() {
        assert_eq!(percent_decode("%2Fhome%2Fuser"), "/home/user");
        assert_eq!(percent_decode("plain"), "plain");
        assert_eq!(percent_decode("%41%42%43"), "ABC");
    }

    // ------------------------------------------------------------------
    // Bonus: sessions sorted newest-first
    // ------------------------------------------------------------------
    #[tokio::test]
    async fn test_sessions_sorted_newest_first() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().to_path_buf();

        // Write two sessions; we can't control mtime easily, but we can at
        // least verify the sort doesn't panic and returns multiple sessions.
        let s1 = root.join("session-alpha");
        let s2 = root.join("session-beta");
        write_format_a_fixture(&s1, &[("user", "Alpha session message")]);
        write_format_a_fixture(&s2, &[("user", "Beta session message")]);

        let sessions = discover_gemini_sessions_in(&root).await;
        assert_eq!(sessions.len(), 2);
        // Verify sorted order: first updated_at >= second updated_at
        assert!(sessions[0].updated_at >= sessions[1].updated_at);
    }
}
