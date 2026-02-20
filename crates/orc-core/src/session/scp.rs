use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use crate::session::types::{
    ConversationMessage, RepoState, Role, SessionContext, SessionSource, ToolActivity,
    ToolType, TokenUsage, UnifiedSession,
};

/// The on-disk SCP format (portable JSON)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScpFile {
    pub version: String,
    pub session_id: String,
    pub source: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub cwd: String,
    pub repo: Option<ScpRepo>,
    pub messages: Vec<ScpMessage>,
    pub tool_activity: Vec<ScpToolActivity>,
    pub files_modified: Vec<String>,
    pub reasoning_notes: Vec<String>,
    pub pending_tasks: Vec<String>,
    pub token_usage: Option<ScpTokenUsage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScpRepo {
    pub branch: Option<String>,
    pub commit: Option<String>,
    pub dirty: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScpMessage {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScpToolActivity {
    #[serde(rename = "type")]
    pub activity_type: String,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default)]
    pub is_write: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScpTokenUsage {
    pub input: u64,
    pub output: u64,
}

fn source_to_str(source: &SessionSource) -> &'static str {
    match source {
        SessionSource::Claude => "claude",
        SessionSource::Codex => "codex",
        SessionSource::Gemini => "gemini",
        SessionSource::Copilot => "copilot",
        SessionSource::OpenCode => "opencode",
        SessionSource::Kimi => "kimi",
        SessionSource::Amp => "amp",
        SessionSource::Droid => "droid",
        SessionSource::Scp => "scp",
    }
}

fn str_to_source(s: &str) -> SessionSource {
    match s {
        "claude" => SessionSource::Claude,
        "codex" => SessionSource::Codex,
        "gemini" => SessionSource::Gemini,
        "copilot" => SessionSource::Copilot,
        "opencode" => SessionSource::OpenCode,
        "kimi" => SessionSource::Kimi,
        "amp" => SessionSource::Amp,
        "droid" => SessionSource::Droid,
        _ => SessionSource::Scp,
    }
}

fn role_to_str(role: &Role) -> &'static str {
    match role {
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::System => "system",
    }
}

fn str_to_role(s: &str) -> Role {
    match s {
        "user" => Role::User,
        "assistant" => Role::Assistant,
        _ => Role::System,
    }
}

fn tool_type_to_str(t: &ToolType) -> &'static str {
    match t {
        ToolType::Shell => "shell",
        ToolType::FileEdit => "file_edit",
        ToolType::FileRead => "file_read",
        ToolType::Grep => "grep",
        ToolType::Search => "search",
        ToolType::Mcp => "mcp",
        ToolType::Patch => "patch",
        ToolType::Other => "other",
    }
}

fn str_to_tool_type(s: &str) -> ToolType {
    match s {
        "shell" => ToolType::Shell,
        "file_edit" => ToolType::FileEdit,
        "file_read" => ToolType::FileRead,
        "grep" => ToolType::Grep,
        "search" => ToolType::Search,
        "mcp" => ToolType::Mcp,
        "patch" => ToolType::Patch,
        _ => ToolType::Other,
    }
}

/// Write a SessionContext to an SCP JSON file at `path`.
pub fn write_scp(path: &Path, ctx: &SessionContext) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let scp = session_context_to_scp(ctx);
    let json = serde_json::to_string_pretty(&scp)?;
    std::fs::write(path, json)?;
    Ok(())
}

/// Read an SCP JSON file and convert it to a SessionContext.
pub fn read_scp(path: &Path) -> Result<SessionContext> {
    let data = std::fs::read_to_string(path)?;
    let scp: ScpFile = serde_json::from_str(&data)?;
    Ok(scp_to_session_context(scp, path))
}

/// Default SCP file path for a given project directory.
pub fn default_scp_path(project_dir: &Path) -> PathBuf {
    project_dir.join(".orc").join("context.json")
}

fn session_context_to_scp(ctx: &SessionContext) -> ScpFile {
    let session = &ctx.session;

    ScpFile {
        version: "1".into(),
        session_id: session.id.clone(),
        source: source_to_str(&session.source).to_string(),
        created_at: session.created_at,
        updated_at: session.updated_at,
        cwd: session.cwd.to_string_lossy().to_string(),
        repo: session.repo.as_ref().map(|r| ScpRepo {
            branch: r.branch.clone(),
            commit: r.commit.clone(),
            dirty: r.dirty,
        }),
        messages: ctx.messages.iter().map(|m| ScpMessage {
            role: role_to_str(&m.role).to_string(),
            content: m.content.clone(),
            model: m.model.clone(),
            timestamp: m.timestamp,
        }).collect(),
        tool_activity: ctx.tool_activity.iter().map(|a| ScpToolActivity {
            activity_type: tool_type_to_str(&a.tool_type).to_string(),
            summary: a.summary.clone(),
            path: a.path.clone(),
            is_write: a.is_write,
        }).collect(),
        files_modified: ctx.files_modified.clone(),
        reasoning_notes: ctx.reasoning_notes.clone(),
        pending_tasks: ctx.pending_tasks.clone(),
        token_usage: ctx.token_usage.as_ref().map(|u| ScpTokenUsage {
            input: u.input,
            output: u.output,
        }),
    }
}

fn scp_to_session_context(scp: ScpFile, source_path: &Path) -> SessionContext {
    let source = str_to_source(&scp.source);

    let session = UnifiedSession {
        id: scp.session_id.clone(),
        source,
        cwd: PathBuf::from(&scp.cwd),
        repo: scp.repo.map(|r| RepoState {
            branch: r.branch,
            commit: r.commit,
            dirty: r.dirty,
        }),
        summary: scp.messages.iter()
            .find(|m| m.role == "user")
            .map(|m| m.content.chars().take(80).collect())
            .unwrap_or_default(),
        updated_at: scp.updated_at,
        created_at: scp.created_at,
        message_count: scp.messages.len(),
        files_modified: vec![],
        original_path: source_path.to_path_buf(),
    };

    let messages: Vec<ConversationMessage> = scp.messages.iter().map(|m| ConversationMessage {
        role: str_to_role(&m.role),
        content: m.content.clone(),
        model: m.model.clone(),
        timestamp: m.timestamp,
    }).collect();

    let tool_activity: Vec<ToolActivity> = scp.tool_activity.iter().map(|a| ToolActivity {
        tool_type: str_to_tool_type(&a.activity_type),
        summary: a.summary.clone(),
        path: a.path.clone(),
        is_write: a.is_write,
    }).collect();

    let files_modified = scp.files_modified;
    let token_usage = scp.token_usage.map(|u| TokenUsage { input: u.input, output: u.output });

    SessionContext {
        session,
        messages,
        tool_activity,
        files_modified,
        reasoning_notes: scp.reasoning_notes,
        pending_tasks: scp.pending_tasks,
        token_usage,
        markdown: String::new(), // regenerated on demand
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use crate::session::types::{ConversationMessage, Role, SessionContext, SessionSource, ToolActivity, ToolType, TokenUsage, UnifiedSession};
    use std::path::PathBuf;

    fn make_ctx() -> SessionContext {
        let session = UnifiedSession {
            id: "scp-test-id".into(),
            source: SessionSource::Claude,
            cwd: PathBuf::from("/Users/me/project"),
            repo: Some(RepoState {
                branch: Some("main".into()),
                commit: Some("abc1234".into()),
                dirty: false,
            }),
            summary: "Add auth middleware".into(),
            updated_at: chrono::DateTime::parse_from_rfc3339("2026-02-20T10:30:00Z").unwrap().into(),
            created_at: chrono::DateTime::parse_from_rfc3339("2026-02-20T10:00:00Z").unwrap().into(),
            message_count: 2,
            files_modified: vec![],
            original_path: PathBuf::from("/tmp/test.jsonl"),
        };

        SessionContext {
            session,
            messages: vec![
                ConversationMessage {
                    role: Role::User,
                    content: "Add auth middleware".into(),
                    model: None,
                    timestamp: None,
                },
                ConversationMessage {
                    role: Role::Assistant,
                    content: "I'll create the middleware.".into(),
                    model: Some("claude-sonnet-4".into()),
                    timestamp: None,
                },
            ],
            tool_activity: vec![
                ToolActivity {
                    tool_type: ToolType::Shell,
                    summary: "npm test -> exit 0".into(),
                    path: None,
                    is_write: false,
                },
            ],
            files_modified: vec!["src/auth.ts".into()],
            reasoning_notes: vec!["Need to handle token refresh".into()],
            pending_tasks: vec!["Write tests".into()],
            token_usage: Some(TokenUsage { input: 1000, output: 500 }),
            markdown: String::new(),
        }
    }

    #[test]
    fn scp_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("context.json");
        let ctx = make_ctx();

        write_scp(&path, &ctx).unwrap();
        let loaded = read_scp(&path).unwrap();

        assert_eq!(loaded.session.id, "scp-test-id");
        assert_eq!(loaded.session.source, SessionSource::Claude);
        assert_eq!(loaded.session.cwd, PathBuf::from("/Users/me/project"));
        assert_eq!(loaded.messages.len(), 2);
        assert_eq!(loaded.messages[0].role, Role::User);
        assert_eq!(loaded.messages[1].role, Role::Assistant);
        assert_eq!(loaded.messages[1].model, Some("claude-sonnet-4".into()));
        assert_eq!(loaded.tool_activity.len(), 1);
        assert_eq!(loaded.tool_activity[0].tool_type, ToolType::Shell);
        assert_eq!(loaded.files_modified, vec!["src/auth.ts"]);
        assert_eq!(loaded.reasoning_notes, vec!["Need to handle token refresh"]);
        assert_eq!(loaded.pending_tasks, vec!["Write tests"]);
        let usage = loaded.token_usage.unwrap();
        assert_eq!(usage.input, 1000);
        assert_eq!(usage.output, 500);
    }

    #[test]
    fn scp_file_is_valid_json() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("context.json");
        let ctx = make_ctx();
        write_scp(&path, &ctx).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(parsed["version"], "1");
        assert_eq!(parsed["source"], "claude");
        assert_eq!(parsed["session_id"], "scp-test-id");
    }

    #[test]
    fn scp_repo_preserved() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("context.json");
        let ctx = make_ctx();
        write_scp(&path, &ctx).unwrap();
        let loaded = read_scp(&path).unwrap();

        let repo = loaded.session.repo.unwrap();
        assert_eq!(repo.branch, Some("main".into()));
        assert_eq!(repo.commit, Some("abc1234".into()));
        assert!(!repo.dirty);
    }

    #[test]
    fn default_scp_path_correct() {
        let project_dir = Path::new("/Users/me/project");
        let expected = PathBuf::from("/Users/me/project/.orc/context.json");
        assert_eq!(default_scp_path(project_dir), expected);
    }

    #[test]
    fn scp_creates_parent_dirs() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".orc").join("context.json");
        let ctx = make_ctx();
        // Parent dir does not exist yet - write_scp should create it
        write_scp(&path, &ctx).unwrap();
        assert!(path.exists());
    }

    #[test]
    fn scp_all_sources_roundtrip() {
        let sources = [
            (SessionSource::Claude, "claude"),
            (SessionSource::Codex, "codex"),
            (SessionSource::Gemini, "gemini"),
            (SessionSource::Copilot, "copilot"),
            (SessionSource::OpenCode, "opencode"),
            (SessionSource::Kimi, "kimi"),
            (SessionSource::Amp, "amp"),
            (SessionSource::Droid, "droid"),
            (SessionSource::Scp, "scp"),
        ];
        for (source, expected_str) in sources {
            assert_eq!(source_to_str(&source), expected_str);
            assert_eq!(str_to_source(expected_str), source);
        }
    }

    #[test]
    fn scp_all_tool_types_roundtrip() {
        let types = [
            (ToolType::Shell, "shell"),
            (ToolType::FileEdit, "file_edit"),
            (ToolType::FileRead, "file_read"),
            (ToolType::Grep, "grep"),
            (ToolType::Search, "search"),
            (ToolType::Mcp, "mcp"),
            (ToolType::Patch, "patch"),
            (ToolType::Other, "other"),
        ];
        for (tool_type, expected_str) in types {
            assert_eq!(tool_type_to_str(&tool_type), expected_str);
            assert_eq!(str_to_tool_type(expected_str), tool_type);
        }
    }

    #[test]
    fn scp_empty_optional_fields() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("context.json");

        let session = UnifiedSession {
            id: "empty-test".into(),
            source: SessionSource::Scp,
            cwd: PathBuf::from("/tmp"),
            repo: None,
            summary: String::new(),
            updated_at: chrono::DateTime::parse_from_rfc3339("2026-02-20T10:00:00Z").unwrap().into(),
            created_at: chrono::DateTime::parse_from_rfc3339("2026-02-20T10:00:00Z").unwrap().into(),
            message_count: 0,
            files_modified: vec![],
            original_path: PathBuf::from("/tmp/none.json"),
        };

        let ctx = SessionContext {
            session,
            messages: vec![],
            tool_activity: vec![],
            files_modified: vec![],
            reasoning_notes: vec![],
            pending_tasks: vec![],
            token_usage: None,
            markdown: String::new(),
        };

        write_scp(&path, &ctx).unwrap();
        let loaded = read_scp(&path).unwrap();

        assert_eq!(loaded.session.id, "empty-test");
        assert!(loaded.session.repo.is_none());
        assert!(loaded.token_usage.is_none());
        assert!(loaded.messages.is_empty());
        assert!(loaded.tool_activity.is_empty());
    }
}
