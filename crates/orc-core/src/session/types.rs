use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionSource {
    Claude,
    Codex,
    Gemini,
    Copilot,
    OpenCode,
    Kimi,
    Amp,
    Droid,
    Scp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoState {
    pub branch: Option<String>,
    pub commit: Option<String>,
    pub dirty: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnifiedSession {
    pub id: String,
    pub source: SessionSource,
    pub cwd: PathBuf,
    pub repo: Option<RepoState>,
    pub summary: String,
    #[serde(default)]
    pub files_modified: Vec<String>,   // top files written/edited this session
    pub updated_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub message_count: usize,
    pub original_path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationMessage {
    pub role: Role,
    pub content: String,
    pub model: Option<String>,
    pub timestamp: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolType {
    Shell,
    FileEdit,
    FileRead,
    Grep,
    Search,
    Mcp,
    Patch,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolActivity {
    pub tool_type: ToolType,
    pub summary: String,
    pub path: Option<String>,
    pub is_write: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input: u64,
    pub output: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionContext {
    pub session: UnifiedSession,
    pub messages: Vec<ConversationMessage>,
    pub tool_activity: Vec<ToolActivity>,
    pub files_modified: Vec<String>,
    pub reasoning_notes: Vec<String>,
    pub pending_tasks: Vec<String>,
    pub token_usage: Option<TokenUsage>,
    pub markdown: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamEvent {
    Text { content: String },
    ToolStart { name: String, input: String },
    ToolEnd { name: String, output: String },
    Reasoning { content: String },
    Status { message: String },
    Error { message: String },
    Done,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_source_roundtrip() {
        let source = SessionSource::Claude;
        let json = serde_json::to_string(&source).unwrap();
        assert_eq!(json, "\"claude\"");
        let back: SessionSource = serde_json::from_str(&json).unwrap();
        assert_eq!(back, SessionSource::Claude);
    }

    #[test]
    fn session_source_all_variants_serialize() {
        let variants = [
            (SessionSource::Claude, "\"claude\""),
            (SessionSource::Codex, "\"codex\""),
            (SessionSource::Gemini, "\"gemini\""),
            (SessionSource::Copilot, "\"copilot\""),
            (SessionSource::OpenCode, "\"open_code\""),
            (SessionSource::Kimi, "\"kimi\""),
            (SessionSource::Amp, "\"amp\""),
            (SessionSource::Droid, "\"droid\""),
            (SessionSource::Scp, "\"scp\""),
        ];
        for (variant, expected) in variants {
            let json = serde_json::to_string(&variant).unwrap();
            assert_eq!(json, expected, "Variant {:?} should serialize to {}", variant, expected);
        }
    }

    #[test]
    fn unified_session_serializes() {
        let session = UnifiedSession {
            id: "abc123".into(),
            source: SessionSource::Codex,
            cwd: "/tmp/test".into(),
            repo: None,
            summary: "Fix auth bug".into(),
            updated_at: chrono::Utc::now(),
            created_at: chrono::Utc::now(),
            message_count: 5,
            files_modified: vec![],
            original_path: "/home/user/.codex/sessions/abc.jsonl".into(),
        };
        let json = serde_json::to_string(&session).unwrap();
        assert!(json.contains("abc123"));
        assert!(json.contains("codex"));
        assert!(json.contains("Fix auth bug"));
    }

    #[test]
    fn stream_event_text_serializes() {
        let event = StreamEvent::Text { content: "hello".into() };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"text\""));
        assert!(json.contains("hello"));
    }
}

#[cfg(test)]
mod tests_files_modified {
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
