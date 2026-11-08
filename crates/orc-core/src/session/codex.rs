use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use crate::session::types::{RepoState, SessionSource, UnifiedSession};

// ---------------------------------------------------------------------------
// Serde structs for flexible JSONL parsing
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct CodexLine {
    timestamp: Option<String>,
    #[serde(rename = "type")]
    line_type: String,
    payload: Option<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// File-name predicate
// ---------------------------------------------------------------------------

/// Returns `true` if `name` looks like a Codex rollout session file.
pub fn is_codex_file(name: &str) -> bool {
    name.starts_with("rollout-") && name.ends_with(".jsonl")
}

// ---------------------------------------------------------------------------
// Internal parsing
// ---------------------------------------------------------------------------

/// Extract the text content from a Codex `content` array.
/// Handles `[{"type":"input_text","text":"..."}, ...]` and
/// `[{"type":"output_text","text":"..."}, ...]`.
fn extract_content_text(content: &serde_json::Value) -> String {
    match content {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(arr) => arr
            .iter()
            .filter_map(|item| {
                let item_type = item.get("type")?.as_str()?;
                if matches!(item_type, "input_text" | "output_text" | "text") {
                    item.get("text")?.as_str().map(|s| s.to_string())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// Parse a single rollout-*.jsonl file into a `UnifiedSession`.
fn parse_codex_file(file_path: &Path) -> Result<UnifiedSession> {
    let file = std::fs::File::open(file_path)?;
    let reader = std::io::BufReader::new(file);

    // Derive a fallback ID from the file stem (e.g. "rollout-abc123" → "abc123")
    let stem = file_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();
    let fallback_id = stem
        .strip_prefix("rollout-")
        .unwrap_or(&stem)
        .to_string();

    let mut session_id = fallback_id.clone();
    let mut cwd = PathBuf::from("/");
    let mut git_branch: Option<String> = None;
    let mut first_user_summary = String::new();
    let mut message_count: usize = 0;
    let mut created_at: Option<DateTime<Utc>> = None;

    for raw_line in reader.lines().take(200) {
        let raw_line = raw_line?;
        if raw_line.is_empty() {
            continue;
        }

        let Ok(codex_line) = serde_json::from_str::<CodexLine>(&raw_line) else {
            continue;
        };

        // Track the earliest timestamp as `created_at`
        if created_at.is_none() {
            if let Some(ts) = &codex_line.timestamp {
                if let Ok(dt) = ts.parse::<DateTime<Utc>>() {
                    created_at = Some(dt);
                }
            }
        }

        match codex_line.line_type.as_str() {
            "session_meta" => {
                if let Some(payload) = &codex_line.payload {
                    // Session ID
                    if let Some(id) = payload.get("id").and_then(|v| v.as_str()) {
                        if !id.is_empty() {
                            session_id = id.to_string();
                        }
                    }
                    // Working directory
                    if let Some(dir) = payload.get("cwd").and_then(|v| v.as_str()) {
                        if !dir.is_empty() {
                            cwd = PathBuf::from(dir);
                        }
                    }
                    // Git branch
                    if let Some(branch) = payload
                        .get("git")
                        .and_then(|g| g.get("branch"))
                        .and_then(|v| v.as_str())
                    {
                        if !branch.is_empty() {
                            git_branch = Some(branch.to_string());
                        }
                    }
                }
            }

            "event_msg" => {
                if let Some(payload) = &codex_line.payload {
                    let role = payload.get("role").and_then(|v| v.as_str()).unwrap_or("");

                    // Count both user and assistant messages
                    if matches!(role, "user" | "assistant") {
                        message_count += 1;
                    }

                    // Capture the first user message as the summary
                    if first_user_summary.is_empty() && role == "user" {
                        if let Some(content) = payload.get("content") {
                            let text = extract_content_text(content);
                            let trimmed = text.trim().to_string();
                            if !trimmed.is_empty()
                                && !trimmed.starts_with('<')
                                && !trimmed.starts_with('/')
                                && !trimmed.contains("Session Handoff")
                            {
                                first_user_summary = trimmed
                                    .replace('\n', " ")
                                    .chars()
                                    .take(80)
                                    .collect();
                            }
                        }
                    }
                }
            }

            _ => {}
        }
    }

    // Determine timestamps from filesystem metadata as fallback
    let meta = std::fs::metadata(file_path)?;
    let updated_at = meta
        .modified()
        .map(DateTime::<Utc>::from)
        .unwrap_or_else(|_| Utc::now());
    let created_at = created_at
        .or_else(|| meta.created().map(DateTime::<Utc>::from).ok())
        .unwrap_or(updated_at);

    Ok(UnifiedSession {
        id: session_id,
        source: SessionSource::Codex,
        cwd,
        repo: git_branch.map(|b| RepoState {
            branch: Some(b),
            commit: None,
            dirty: false,
        }),
        summary: first_user_summary,
        updated_at,
        created_at,
        message_count,
        files_modified: vec![],
        original_path: file_path.to_path_buf(),
    })
}

// ---------------------------------------------------------------------------
// Public discovery functions
// ---------------------------------------------------------------------------

/// Walk `root` recursively and parse every `rollout-*.jsonl` file found.
pub fn discover_codex_sessions_in(root: &Path) -> Result<Vec<UnifiedSession>> {
    let mut sessions = Vec::new();

    if !root.exists() {
        return Ok(sessions);
    }

    // Use a simple iterative DFS via a stack to avoid pulling in walkdir
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.is_file() {
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("");
                if is_codex_file(name) {
                    if let Ok(session) = parse_codex_file(&path) {
                        sessions.push(session);
                    }
                }
            }
        }
    }

    sessions.sort_by_key(|s| std::cmp::Reverse(s.updated_at)); // newest first
    Ok(sessions)
}

/// Discover all Codex sessions from the default location: `~/.codex/sessions`.
pub fn discover_codex_sessions() -> Result<Vec<UnifiedSession>> {
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("no home directory found"))?;
    let root = home.join(".codex").join("sessions");
    discover_codex_sessions_in(&root)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Write a four-line fixture rollout-abc123.jsonl under `sessions_dir`.
    fn write_codex_fixture(sessions_dir: &Path) -> PathBuf {
        fs::create_dir_all(sessions_dir).unwrap();
        let file_path = sessions_dir.join("rollout-abc123.jsonl");
        let lines = [
            // 1. session_meta
            r#"{"timestamp":"2026-02-20T10:00:00Z","type":"session_meta","payload":{"id":"ses_abc123","cwd":"/Users/me/project","git":{"branch":"main","repository_url":"https://github.com/me/project"}}}"#,
            // 2. user message
            r#"{"timestamp":"2026-02-20T10:01:00Z","type":"event_msg","payload":{"role":"user","content":[{"type":"input_text","text":"Fix the auth bug"}]}}"#,
            // 3. assistant message
            r#"{"timestamp":"2026-02-20T10:02:00Z","type":"event_msg","payload":{"role":"assistant","content":[{"type":"output_text","text":"I will look into the auth bug now."}]}}"#,
            // 4. second user message
            r#"{"timestamp":"2026-02-20T10:03:00Z","type":"event_msg","payload":{"role":"user","content":[{"type":"input_text","text":"Now add tests"}]}}"#,
        ];
        fs::write(&file_path, lines.join("\n")).unwrap();
        file_path
    }

    // ------------------------------------------------------------------
    // is_codex_file
    // ------------------------------------------------------------------

    #[test]
    fn is_codex_file_accepts_valid_names() {
        assert!(is_codex_file("rollout-abc123.jsonl"));
        assert!(is_codex_file("rollout-some-long-id-here.jsonl"));
        assert!(is_codex_file("rollout-.jsonl")); // minimal edge case
    }

    #[test]
    fn is_codex_file_rejects_invalid_names() {
        assert!(!is_codex_file("session.jsonl"));
        assert!(!is_codex_file("rollout-abc123.json")); // wrong extension
        assert!(!is_codex_file("uuid-abc.jsonl"));      // wrong prefix
        assert!(!is_codex_file("debug.log"));
        assert!(!is_codex_file(""));
    }

    // ------------------------------------------------------------------
    // discover_codex_sessions_in — happy path
    // ------------------------------------------------------------------

    #[test]
    fn discovers_session_from_fixture() {
        let tmp = TempDir::new().unwrap();
        let sessions_dir = tmp.path().join("sessions");
        write_codex_fixture(&sessions_dir);

        let sessions = discover_codex_sessions_in(&sessions_dir).unwrap();
        assert!(!sessions.is_empty(), "Should discover at least one session");
    }

    #[test]
    fn session_id_from_payload() {
        let tmp = TempDir::new().unwrap();
        let sessions_dir = tmp.path().join("sessions");
        write_codex_fixture(&sessions_dir);

        let sessions = discover_codex_sessions_in(&sessions_dir).unwrap();
        assert_eq!(sessions[0].id, "ses_abc123", "ID should come from session_meta payload");
    }

    #[test]
    fn cwd_from_payload() {
        let tmp = TempDir::new().unwrap();
        let sessions_dir = tmp.path().join("sessions");
        write_codex_fixture(&sessions_dir);

        let sessions = discover_codex_sessions_in(&sessions_dir).unwrap();
        assert_eq!(
            sessions[0].cwd,
            PathBuf::from("/Users/me/project"),
            "cwd should come from session_meta payload"
        );
    }

    #[test]
    fn branch_from_git_payload() {
        let tmp = TempDir::new().unwrap();
        let sessions_dir = tmp.path().join("sessions");
        write_codex_fixture(&sessions_dir);

        let sessions = discover_codex_sessions_in(&sessions_dir).unwrap();
        let repo = sessions[0].repo.as_ref().expect("repo should be Some");
        assert_eq!(repo.branch.as_deref(), Some("main"));
    }

    #[test]
    fn summary_is_first_user_message() {
        let tmp = TempDir::new().unwrap();
        let sessions_dir = tmp.path().join("sessions");
        write_codex_fixture(&sessions_dir);

        let sessions = discover_codex_sessions_in(&sessions_dir).unwrap();
        assert!(
            sessions[0].summary.contains("Fix the auth bug"),
            "Summary should be the first user message, got: {}",
            sessions[0].summary
        );
    }

    #[test]
    fn message_count_includes_user_and_assistant() {
        let tmp = TempDir::new().unwrap();
        let sessions_dir = tmp.path().join("sessions");
        write_codex_fixture(&sessions_dir);

        let sessions = discover_codex_sessions_in(&sessions_dir).unwrap();
        // Fixture has: 1 user + 1 assistant + 1 user = 3 event_msg lines
        assert_eq!(sessions[0].message_count, 3, "message_count should be 3");
    }

    #[test]
    fn source_is_codex() {
        let tmp = TempDir::new().unwrap();
        let sessions_dir = tmp.path().join("sessions");
        write_codex_fixture(&sessions_dir);

        let sessions = discover_codex_sessions_in(&sessions_dir).unwrap();
        assert_eq!(sessions[0].source, SessionSource::Codex);
    }

    // ------------------------------------------------------------------
    // Fallback behaviour when session_meta is absent
    // ------------------------------------------------------------------

    #[test]
    fn fallback_id_from_filename_stem() {
        let tmp = TempDir::new().unwrap();
        let sessions_dir = tmp.path().join("sessions");
        fs::create_dir_all(&sessions_dir).unwrap();

        let file_path = sessions_dir.join("rollout-xyz789.jsonl");
        // Only an event_msg, no session_meta
        let line = r#"{"timestamp":"2026-02-20T09:00:00Z","type":"event_msg","payload":{"role":"user","content":[{"type":"input_text","text":"Hello world"}]}}"#;
        fs::write(&file_path, line).unwrap();

        let sessions = discover_codex_sessions_in(&sessions_dir).unwrap();
        assert!(!sessions.is_empty());
        assert_eq!(sessions[0].id, "xyz789", "Fallback ID should strip 'rollout-' prefix");
    }

    // ------------------------------------------------------------------
    // Non-existent root returns empty vec without error
    // ------------------------------------------------------------------

    #[test]
    fn non_existent_root_returns_empty() {
        let tmp = TempDir::new().unwrap();
        let missing = tmp.path().join("does_not_exist");
        let sessions = discover_codex_sessions_in(&missing).unwrap();
        assert!(sessions.is_empty());
    }

    // ------------------------------------------------------------------
    // Non-rollout files are ignored
    // ------------------------------------------------------------------

    #[test]
    fn ignores_non_rollout_files() {
        let tmp = TempDir::new().unwrap();
        let sessions_dir = tmp.path().join("sessions");
        fs::create_dir_all(&sessions_dir).unwrap();

        // Write a file that does NOT match the rollout-*.jsonl pattern
        fs::write(
            sessions_dir.join("unrelated.jsonl"),
            r#"{"type":"event_msg","payload":{"role":"user","content":[{"type":"input_text","text":"ignored"}]}}"#,
        )
        .unwrap();

        let sessions = discover_codex_sessions_in(&sessions_dir).unwrap();
        assert!(sessions.is_empty(), "Non-rollout files must be ignored");
    }

    // ------------------------------------------------------------------
    // extract_content_text helper
    // ------------------------------------------------------------------

    #[test]
    fn extract_input_text_type() {
        let val: serde_json::Value = serde_json::json!([
            {"type": "input_text", "text": "hello from user"}
        ]);
        assert_eq!(extract_content_text(&val), "hello from user");
    }

    #[test]
    fn extract_output_text_type() {
        let val: serde_json::Value = serde_json::json!([
            {"type": "output_text", "text": "response from assistant"}
        ]);
        assert_eq!(extract_content_text(&val), "response from assistant");
    }

    #[test]
    fn extract_skips_unknown_content_types() {
        let val: serde_json::Value = serde_json::json!([
            {"type": "image_url", "url": "https://example.com/img.png"},
            {"type": "input_text", "text": "only this"}
        ]);
        assert_eq!(extract_content_text(&val), "only this");
    }

    #[test]
    fn extract_plain_string_content() {
        let val = serde_json::Value::String("plain string".to_string());
        assert_eq!(extract_content_text(&val), "plain string");
    }

    // ------------------------------------------------------------------
    // Recursive walk finds files in subdirectories
    // ------------------------------------------------------------------

    #[test]
    fn discovers_sessions_in_subdirectories() {
        let tmp = TempDir::new().unwrap();
        let sessions_dir = tmp.path().join("sessions");
        let sub_dir = sessions_dir.join("2026").join("02");
        fs::create_dir_all(&sub_dir).unwrap();

        let file_path = sub_dir.join("rollout-nested123.jsonl");
        let lines = [
            r#"{"timestamp":"2026-02-20T08:00:00Z","type":"session_meta","payload":{"id":"ses_nested","cwd":"/tmp","git":{}}}"#,
            r#"{"timestamp":"2026-02-20T08:01:00Z","type":"event_msg","payload":{"role":"user","content":[{"type":"input_text","text":"Nested session task"}]}}"#,
        ];
        fs::write(&file_path, lines.join("\n")).unwrap();

        let sessions = discover_codex_sessions_in(&sessions_dir).unwrap();
        assert!(!sessions.is_empty(), "Should find sessions in subdirectories");
        assert_eq!(sessions[0].id, "ses_nested");
    }
}
