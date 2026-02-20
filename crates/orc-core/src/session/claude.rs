use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use crate::session::types::{SessionSource, UnifiedSession};

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct ClaudeMessage {
    #[serde(rename = "type")]
    msg_type: String,
    #[serde(default)]
    uuid: String,
    #[serde(default)]
    timestamp: Option<String>,
    #[serde(rename = "sessionId", default)]
    session_id: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(rename = "gitBranch", default)]
    git_branch: Option<String>,
    #[serde(default)]
    message: Option<ClaudeMessageContent>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct ClaudeMessageContent {
    role: Option<String>,
    content: Option<serde_json::Value>,
}

fn extract_text(content: &serde_json::Value) -> String {
    match content {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(arr) => {
            arr.iter()
                .filter_map(|item| {
                    if item.get("type")?.as_str()? == "text" {
                        item.get("text")?.as_str().map(|s| s.to_string())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
        _ => String::new(),
    }
}

fn is_uuid_jsonl(filename: &str) -> bool {
    // Matches: 8-4-4-4-12 hex chars + .jsonl
    if !filename.ends_with(".jsonl") {
        return false;
    }
    let stem = &filename[..filename.len() - 6];
    let parts: Vec<&str> = stem.split('-').collect();
    if parts.len() != 5 {
        return false;
    }
    let lens = [8, 4, 4, 4, 12];
    parts.iter().zip(lens.iter()).all(|(p, &l)| {
        p.len() == l && p.chars().all(|c| c.is_ascii_hexdigit())
    })
}

/// Scan raw JSONL lines for Write/Edit tool_use blocks.
/// Returns up to 5 unique file paths relative to cwd when possible.
fn extract_files_modified(lines: &[String], cwd: &Path) -> Vec<String> {
    const WRITE_TOOLS: &[&str] = &[
        "Write", "Edit", "MultiEdit", "WriteFile", "EditFile", "write_file", "edit_file",
    ];
    let mut files: Vec<String> = Vec::new();

    for line in lines {
        if files.len() >= 5 {
            break;
        }
        let Ok(val) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(content) = val
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_array())
        else {
            continue;
        };
        for item in content {
            if files.len() >= 5 {
                break;
            }
            let Some(name) = item.get("name").and_then(|n| n.as_str()) else {
                continue;
            };
            if !WRITE_TOOLS.contains(&name) {
                continue;
            }
            let input = item.get("input").and_then(|i| i.as_object());
            let path_str = input
                .and_then(|m| m.get("file_path").or_else(|| m.get("path")))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if path_str.is_empty() {
                continue;
            }
            // Make relative to cwd if possible
            let display = Path::new(path_str)
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

/// Discover and parse all Claude sessions from a root directory.
/// root is typically ~/.claude/projects
pub async fn discover_claude_sessions_in(root: &Path) -> Result<Vec<UnifiedSession>> {
    let mut sessions = Vec::new();

    if !root.exists() {
        return Ok(sessions);
    }

    // Walk subdirectories (project slugs)
    let project_dirs = std::fs::read_dir(root)?;
    for entry in project_dirs.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        // Walk session files in this project dir
        if let Ok(files) = std::fs::read_dir(&path) {
            for file_entry in files.flatten() {
                let file_path = file_entry.path();
                let name = file_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("");
                if !is_uuid_jsonl(name) {
                    continue;
                }
                // Parse session metadata
                if let Ok(session) = parse_session_file(&file_path) {
                    sessions.push(session);
                }
            }
        }
    }

    sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(sessions)
}

fn parse_session_file(file_path: &Path) -> Result<UnifiedSession> {
    let file = std::fs::File::open(file_path)?;
    let reader = std::io::BufReader::new(file);

    let mut session_id = file_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    let file_stem = session_id.clone();
    let mut cwd = PathBuf::from("/");
    let mut git_branch: Option<String> = None;
    let mut first_user_message = String::new();
    let mut message_count = 0usize;
    // Collect all raw lines for files_modified extraction (no second file read)
    let mut raw_lines: Vec<String> = Vec::new();

    for line in reader.lines().take(100) {
        let line = line?;
        if line.is_empty() {
            continue;
        }
        raw_lines.push(line.clone());

        let Ok(msg) = serde_json::from_str::<ClaudeMessage>(&line) else {
            continue;
        };

        if let Some(sid) = &msg.session_id {
            if session_id.is_empty() || session_id == file_stem {
                session_id = sid.clone();
            }
        }
        if let Some(c) = &msg.cwd {
            if !c.is_empty() {
                cwd = PathBuf::from(c);
            }
        }
        if let Some(b) = &msg.git_branch {
            if !b.is_empty() {
                git_branch = Some(b.clone());
            }
        }

        if msg.msg_type == "user" || msg.msg_type == "assistant" {
            message_count += 1;
        }

        if first_user_message.is_empty() && msg.msg_type == "user" {
            if let Some(content) = msg.message.as_ref().and_then(|m| m.content.as_ref()) {
                let text = extract_text(content);
                // Skip commands, meta content, handoff markers
                if !text.starts_with('<')
                    && !text.starts_with('/')
                    && !text.contains("Session Handoff")
                    && !text.is_empty()
                {
                    first_user_message = text
                        .replace('\n', " ")
                        .trim()
                        .chars()
                        .take(80)
                        .collect();
                }
            }
        }
    }

    // Extract files modified from the already-collected raw lines
    let files_modified = extract_files_modified(&raw_lines, &cwd);

    let meta = std::fs::metadata(file_path)?;
    let updated_at = meta
        .modified()
        .map(|t| DateTime::<Utc>::from(t))
        .unwrap_or_else(|_| Utc::now());
    let created_at = meta
        .created()
        .map(|t| DateTime::<Utc>::from(t))
        .unwrap_or(updated_at);

    Ok(UnifiedSession {
        id: session_id,
        source: SessionSource::Claude,
        cwd,
        repo: git_branch.map(|b| crate::session::types::RepoState {
            branch: Some(b),
            commit: None,
            dirty: false,
        }),
        summary: first_user_message,
        updated_at,
        created_at,
        message_count,
        files_modified,
        original_path: file_path.to_path_buf(),
    })
}

/// Default entry: uses ~/.claude/projects
pub async fn discover_claude_sessions() -> Result<Vec<UnifiedSession>> {
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("no home dir"))?;
    let root = home.join(".claude").join("projects");
    discover_claude_sessions_in(&root).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_claude_fixture(dir: &Path) -> PathBuf {
        let project_dir = dir.join(".claude").join("projects").join("-Users-me-project");
        fs::create_dir_all(&project_dir).unwrap();
        // Use a valid UUID filename
        let session_file =
            project_dir.join("abc12345-0000-0000-0000-000000000001.jsonl");
        let lines = vec![
            r#"{"type":"user","uuid":"u1","timestamp":"2026-02-20T10:00:00Z","sessionId":"abc12345-0000-0000-0000-000000000001","cwd":"/Users/me/project","gitBranch":"main","message":{"role":"user","content":"Fix the auth bug"}}"#,
            r#"{"type":"assistant","uuid":"u2","timestamp":"2026-02-20T10:01:00Z","message":{"role":"assistant","content":"I'll fix that."}}"#,
            r#"{"type":"user","uuid":"u3","timestamp":"2026-02-20T10:02:00Z","message":{"role":"user","content":"Now add tests"}}"#,
            r#"{"type":"assistant","uuid":"u4","timestamp":"2026-02-20T10:03:00Z","message":{"role":"assistant","content":"Done, tests added."}}"#,
        ];
        fs::write(&session_file, lines.join("\n")).unwrap();
        dir.to_path_buf()
    }

    #[tokio::test]
    async fn discovers_claude_sessions() {
        let tmp = TempDir::new().unwrap();
        write_claude_fixture(tmp.path());
        let claude_dir = tmp.path().join(".claude").join("projects");
        let sessions = discover_claude_sessions_in(&claude_dir).await.unwrap();
        assert!(!sessions.is_empty(), "Should find at least one session");
        assert_eq!(sessions[0].source, SessionSource::Claude);
        assert!(
            sessions[0].summary.contains("Fix the auth bug"),
            "Summary should be first user message, got: {}",
            sessions[0].summary
        );
    }

    #[test]
    fn uuid_filename_detection() {
        assert!(is_uuid_jsonl(
            "abc12345-0000-0000-0000-000000000001.jsonl"
        ));
        assert!(!is_uuid_jsonl("debug.jsonl"));
        assert!(!is_uuid_jsonl("sessions.jsonl"));
        assert!(!is_uuid_jsonl("abc12345-0000-0000-0000.jsonl")); // wrong parts count
    }

    #[test]
    fn extract_text_from_string() {
        let val = serde_json::Value::String("hello world".to_string());
        assert_eq!(extract_text(&val), "hello world");
    }

    #[test]
    fn extract_text_from_array() {
        let val: serde_json::Value = serde_json::json!([
            {"type": "text", "text": "part one"},
            {"type": "image", "url": "http://example.com/img.png"},
            {"type": "text", "text": "part two"}
        ]);
        assert_eq!(extract_text(&val), "part one\npart two");
    }

    #[test]
    fn skips_command_messages() {
        let tmp = TempDir::new().unwrap();
        let project_dir = tmp
            .path()
            .join(".claude")
            .join("projects")
            .join("-Users-me-proj2");
        fs::create_dir_all(&project_dir).unwrap();
        let session_file =
            project_dir.join("deadbeef-1111-1111-1111-aabbccddeeff.jsonl");
        let lines = vec![
            // first user message starts with '/' — should be skipped
            r#"{"type":"user","uuid":"u1","sessionId":"deadbeef-1111-1111-1111-aabbccddeeff","cwd":"/tmp","message":{"role":"user","content":"/help"}}"#,
            // second user message is normal — should be picked up
            r#"{"type":"user","uuid":"u2","message":{"role":"user","content":"Build the feature"}}"#,
        ];
        fs::write(&session_file, lines.join("\n")).unwrap();

        // Use tokio runtime synchronously via block_on
        let rt = tokio::runtime::Runtime::new().unwrap();
        let sessions = rt
            .block_on(discover_claude_sessions_in(
                &tmp.path().join(".claude").join("projects"),
            ))
            .unwrap();

        assert!(!sessions.is_empty());
        assert!(
            sessions[0].summary.contains("Build the feature"),
            "Should skip /help and pick up next user message, got: {}",
            sessions[0].summary
        );
    }

    #[test]
    fn message_count_includes_user_and_assistant() {
        let tmp = TempDir::new().unwrap();
        write_claude_fixture(tmp.path());
        let claude_dir = tmp.path().join(".claude").join("projects");

        let rt = tokio::runtime::Runtime::new().unwrap();
        let sessions = rt
            .block_on(discover_claude_sessions_in(&claude_dir))
            .unwrap();

        assert!(!sessions.is_empty());
        // fixture has 2 user + 2 assistant = 4 messages
        assert_eq!(sessions[0].message_count, 4);
    }

    #[test]
    fn git_branch_captured() {
        let tmp = TempDir::new().unwrap();
        write_claude_fixture(tmp.path());
        let claude_dir = tmp.path().join(".claude").join("projects");

        let rt = tokio::runtime::Runtime::new().unwrap();
        let sessions = rt
            .block_on(discover_claude_sessions_in(&claude_dir))
            .unwrap();

        assert!(!sessions.is_empty());
        let repo = sessions[0].repo.as_ref().expect("repo should be Some");
        assert_eq!(repo.branch.as_deref(), Some("main"));
    }

    #[test]
    fn test_extract_files_modified_from_write_tools() {
        use std::io::Write;

        let dir = TempDir::new().unwrap();
        let proj = dir.path().join("projects").join("-tmp-myproject");
        std::fs::create_dir_all(&proj).unwrap();

        let session_id = "aaaaaaaa-0000-0000-0000-000000000000";
        let file = proj.join(format!("{}.jsonl", session_id));

        let mut f = std::fs::File::create(&file).unwrap();
        writeln!(f, r#"{{"type":"summary","sessionId":"{}","cwd":"/tmp/myproject"}}"#, session_id).unwrap();
        writeln!(f, r#"{{"type":"user","uuid":"u1","timestamp":"2026-01-01T00:00:00Z","message":{{"role":"user","content":"add auth"}}}}"#).unwrap();
        writeln!(f, r#"{{"type":"assistant","uuid":"a1","timestamp":"2026-01-01T00:00:01Z","message":{{"role":"assistant","content":[{{"type":"tool_use","id":"t1","name":"Write","input":{{"file_path":"/tmp/myproject/src/auth.ts","content":"..."}}}}]}}}}"#).unwrap();
        writeln!(f, r#"{{"type":"assistant","uuid":"a2","timestamp":"2026-01-01T00:00:02Z","message":{{"role":"assistant","content":[{{"type":"tool_use","id":"t2","name":"Edit","input":{{"file_path":"/tmp/myproject/src/middleware.ts","old_string":"x","new_string":"y"}}}}]}}}}"#).unwrap();

        let sessions = tokio::runtime::Builder::new_current_thread()
            .enable_all().build().unwrap()
            .block_on(discover_claude_sessions_in(&dir.path().join("projects")));

        // sessions is Result<Vec<UnifiedSession>>
        let sessions = sessions.unwrap();
        assert_eq!(sessions.len(), 1);
        let s = &sessions[0];
        assert!(
            s.files_modified.contains(&"src/auth.ts".to_string()) ||
            s.files_modified.contains(&"/tmp/myproject/src/auth.ts".to_string()),
            "expected auth.ts in files_modified, got: {:?}", s.files_modified
        );
        assert!(s.files_modified.len() <= 5);
    }

    #[test]
    fn test_files_modified_capped_at_five() {
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
            .block_on(discover_claude_sessions_in(&dir.path().join("projects")));

        let sessions = sessions.unwrap();
        assert_eq!(sessions.len(), 1);
        assert!(
            sessions[0].files_modified.len() <= 5,
            "files_modified should be capped at 5, got {}", sessions[0].files_modified.len()
        );
    }

    #[test]
    fn extract_files_modified_unit() {
        let lines = vec![
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"Write","input":{"file_path":"/repo/src/main.rs","content":"..."}}]}}"#.to_string(),
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t2","name":"Edit","input":{"file_path":"/repo/src/lib.rs","old_string":"x","new_string":"y"}}]}}"#.to_string(),
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t3","name":"Bash","input":{"command":"cargo build"}}]}}"#.to_string(),
        ];
        let cwd = Path::new("/repo");
        let files = extract_files_modified(&lines, cwd);
        assert_eq!(files, vec!["src/main.rs", "src/lib.rs"]);
    }

    #[test]
    fn extract_files_modified_deduplicates() {
        let lines = vec![
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"Write","input":{"file_path":"/repo/src/main.rs","content":"v1"}}]}}"#.to_string(),
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t2","name":"Edit","input":{"file_path":"/repo/src/main.rs","old_string":"x","new_string":"y"}}]}}"#.to_string(),
        ];
        let cwd = Path::new("/repo");
        let files = extract_files_modified(&lines, cwd);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0], "src/main.rs");
    }
}
