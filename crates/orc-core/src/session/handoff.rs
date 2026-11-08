use crate::session::types::{Role, SessionContext, SessionSource, ToolActivity, ToolType};

/// Format a number with thousands-separator commas (e.g. 45230 → "45,230").
fn fmt_commas(n: u64) -> String {
    let s = n.to_string();
    let mut result = String::new();
    let chars: Vec<char> = s.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        if i > 0 && (chars.len() - i).is_multiple_of(3) {
            result.push(',');
        }
        result.push(*c);
    }
    result
}

/// Generate a structured markdown handoff document from a SessionContext.
/// Secrets are redacted: the document is pasted into another vendor's CLI.
pub fn generate_handoff_markdown(ctx: &SessionContext) -> String {
    let session = &ctx.session;
    let redactor = crate::redact::Redactor::for_dir(&session.cwd);
    let mut md = String::new();

    // Header
    md.push_str("# Session Handoff\n\n");

    // Overview table
    md.push_str("## Overview\n\n");
    md.push_str("| Detail | Value |\n");
    md.push_str("|--------|-------|\n");

    let source_name = source_label(&session.source);
    md.push_str(&format!("| Previous tool | {} |\n", source_name));
    md.push_str(&format!("| Working directory | `{}` |\n", session.cwd.display()));

    // Project name (last component of cwd)
    let project = session.cwd.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("?");
    md.push_str(&format!("| Project | {} |\n", project));

    if !session.summary.is_empty() {
        let truncated: String = session.summary.chars().take(80).collect();
        md.push_str(&format!("| Last task | {} |\n", truncated));
    }

    if let Some(repo) = &session.repo {
        if let Some(branch) = &repo.branch {
            md.push_str(&format!("| Git branch | {} |\n", branch));
        }
        if let Some(commit) = &repo.commit {
            let short: String = commit.chars().take(8).collect();
            md.push_str(&format!("| Last commit | {} |\n", short));
        }
    }

    md.push_str(&format!("| Message count | {} |\n", session.message_count));

    if let Some(usage) = &ctx.token_usage {
        md.push_str(&format!(
            "| Token usage | {} input, {} output |\n",
            fmt_commas(usage.input),
            fmt_commas(usage.output)
        ));
    }

    md.push('\n');

    // Tool Activity
    if !ctx.tool_activity.is_empty() {
        md.push_str("## Tool Activity\n\n");
        // Group by tool type
        let mut grouped: std::collections::HashMap<String, Vec<&ToolActivity>> =
            std::collections::HashMap::new();
        for activity in &ctx.tool_activity {
            let key = tool_type_label(&activity.tool_type).to_string();
            grouped.entry(key).or_default().push(activity);
        }
        let mut keys: Vec<String> = grouped.keys().cloned().collect();
        keys.sort();
        for key in &keys {
            if let Some(activities) = grouped.get(key) {
                let count = activities.len();
                let samples: Vec<&str> = activities.iter()
                    .take(3)
                    .map(|a| a.summary.as_str())
                    .collect();
                md.push_str(&format!(
                    "- **{}** (×{}): {}\n",
                    key,
                    count,
                    samples.join(" · ")
                ));
            }
        }
        md.push('\n');
    }

    // Reasoning notes
    if !ctx.reasoning_notes.is_empty() {
        md.push_str("## Session Notes\n\n");
        for note in &ctx.reasoning_notes {
            md.push_str(&format!("- 💭 {}\n", note));
        }
        md.push('\n');
    }

    // Pending tasks
    if !ctx.pending_tasks.is_empty() {
        md.push_str("## Pending Tasks\n\n");
        for task in &ctx.pending_tasks {
            md.push_str(&format!("- [ ] {}\n", task));
        }
        md.push('\n');
    }

    // Files modified
    if !ctx.files_modified.is_empty() {
        md.push_str("## Files Modified\n\n");
        let mut sorted = ctx.files_modified.clone();
        sorted.sort();
        sorted.dedup();
        for file in &sorted {
            md.push_str(&format!("- `{}`\n", file));
        }
        md.push('\n');
    }

    // Recent conversation (last 10 messages)
    if !ctx.messages.is_empty() {
        md.push_str("## Recent Conversation\n\n");
        for msg in ctx.messages.iter().rev().take(10).rev() {
            let role_label = match msg.role {
                Role::User => "**User**",
                Role::Assistant => "**Assistant**",
                Role::System => "**System**",
            };
            // Redact before cutting, so a cut cannot split a secret past recognition.
            let content: String = redactor.clean(&msg.content).chars().take(500).collect();
            md.push_str(&format!("### {}\n\n{}\n\n", role_label, content));
        }
    }

    // Closing directive
    md.push_str("---\n\n");
    md.push_str("Pick up from where the previous session left off. Review the context above and continue the work.\n");

    let mut md = redactor.clean(&md);
    if crate::redact::has_redactions(&md) {
        md.push_str(&format!("\n{}\n", crate::redact::note()));
    }
    md
}

fn source_label(source: &SessionSource) -> &'static str {
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

fn tool_type_label(tool_type: &ToolType) -> &'static str {
    match tool_type {
        ToolType::Shell => "Bash",
        ToolType::FileEdit => "Edit",
        ToolType::FileRead => "Read",
        ToolType::Grep => "Grep",
        ToolType::Search => "Search",
        ToolType::Mcp => "MCP",
        ToolType::Patch => "Patch",
        ToolType::Other => "Other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::types::{
        ConversationMessage, RepoState, Role, SessionContext, SessionSource, ToolActivity,
        ToolType, TokenUsage, UnifiedSession,
    };
    use std::path::PathBuf;

    fn make_test_context() -> SessionContext {
        let session = UnifiedSession {
            id: "test-session-id".into(),
            source: SessionSource::Claude,
            cwd: PathBuf::from("/Users/me/my-project"),
            repo: Some(RepoState {
                branch: Some("main".into()),
                commit: Some("a1b2c3d4".into()),
                dirty: true,
            }),
            summary: "Fix the authentication bug".into(),
            updated_at: chrono::Utc::now(),
            created_at: chrono::Utc::now(),
            message_count: 10,
            files_modified: vec![],
            original_path: PathBuf::from("/home/.claude/projects/test/test.jsonl"),
        };

        SessionContext {
            session,
            messages: vec![
                ConversationMessage {
                    role: Role::User,
                    content: "Fix the auth bug please".into(),
                    model: None,
                    timestamp: None,
                },
                ConversationMessage {
                    role: Role::Assistant,
                    content: "I'll fix it right away.".into(),
                    model: Some("claude-sonnet-4".into()),
                    timestamp: None,
                },
            ],
            tool_activity: vec![
                ToolActivity {
                    tool_type: ToolType::Shell,
                    summary: "$ npm test → exit 0".into(),
                    path: None,
                    is_write: false,
                },
                ToolActivity {
                    tool_type: ToolType::FileEdit,
                    summary: "edit src/auth.ts".into(),
                    path: Some("src/auth.ts".into()),
                    is_write: true,
                },
            ],
            files_modified: vec!["src/auth.ts".into(), "src/middleware.ts".into()],
            reasoning_notes: vec!["Need to handle token refresh edge case".into()],
            pending_tasks: vec!["Add tests for auth middleware".into()],
            token_usage: Some(TokenUsage { input: 45230, output: 12847 }),
            markdown: String::new(),
        }
    }

    #[test]
    fn handoff_removes_secrets_from_conversation_and_notes() {
        let mut ctx = make_test_context();
        let filler = "x ".repeat(240);
        ctx.messages[0].content = format!("{filler}export OPENAI_API_KEY=sk-proj-abcdefghijklmnopqrstuvwxyz0123456789");
        ctx.reasoning_notes.push("db at postgres://app:s3cretPassw0rd@db:5432/app".into());
        let md = generate_handoff_markdown(&ctx);
        assert!(!md.contains("sk-proj-abcdef"), "{md}");
        assert!(!md.contains("s3cretPassw0rd"), "{md}");
        assert!(md.contains("[REDACTED:"), "{md}");
        assert!(md.contains(crate::redact::note()));
    }

    #[test]
    fn handoff_contains_overview_table() {
        let ctx = make_test_context();
        let md = generate_handoff_markdown(&ctx);
        assert!(md.contains("## Overview"), "Should have Overview section");
        assert!(md.contains("claude"), "Should mention source tool");
        assert!(md.contains("my-project"), "Should mention project name");
        assert!(md.contains("Fix the authentication bug"), "Should include summary");
        assert!(md.contains("main"), "Should include git branch");
    }

    #[test]
    fn handoff_contains_tool_activity() {
        let ctx = make_test_context();
        let md = generate_handoff_markdown(&ctx);
        assert!(md.contains("## Tool Activity"), "Should have Tool Activity section");
        assert!(md.contains("Bash"), "Should mention Bash tool");
        assert!(md.contains("Edit"), "Should mention Edit tool");
    }

    #[test]
    fn handoff_contains_reasoning_notes() {
        let ctx = make_test_context();
        let md = generate_handoff_markdown(&ctx);
        assert!(md.contains("Session Notes"), "Should have Session Notes section");
        assert!(md.contains("token refresh"), "Should include reasoning note");
    }

    #[test]
    fn handoff_contains_recent_conversation() {
        let ctx = make_test_context();
        let md = generate_handoff_markdown(&ctx);
        assert!(md.contains("Recent Conversation"), "Should have conversation section");
        assert!(md.contains("Fix the auth bug"), "Should include user message");
        assert!(md.contains("I'll fix it right away"), "Should include assistant message");
    }

    #[test]
    fn handoff_contains_files_modified() {
        let ctx = make_test_context();
        let md = generate_handoff_markdown(&ctx);
        assert!(md.contains("Files Modified"), "Should have files section");
        assert!(md.contains("src/auth.ts"), "Should list modified files");
    }

    #[test]
    fn handoff_contains_pending_tasks() {
        let ctx = make_test_context();
        let md = generate_handoff_markdown(&ctx);
        assert!(md.contains("Pending Tasks"), "Should have pending tasks");
        assert!(md.contains("Add tests for auth middleware"), "Should list pending tasks");
    }

    #[test]
    fn handoff_token_usage_formatted_with_commas() {
        let ctx = make_test_context();
        let md = generate_handoff_markdown(&ctx);
        assert!(md.contains("45,230"), "Input token count should be comma-formatted");
        assert!(md.contains("12,847"), "Output token count should be comma-formatted");
    }

    #[test]
    fn handoff_files_sorted_and_deduped() {
        let mut ctx = make_test_context();
        ctx.files_modified = vec![
            "src/z.ts".into(),
            "src/auth.ts".into(),
            "src/auth.ts".into(), // duplicate
            "src/a.ts".into(),
        ];
        let md = generate_handoff_markdown(&ctx);
        let files_section_start = md.find("## Files Modified").unwrap();
        let files_section = &md[files_section_start..];
        let a_pos = files_section.find("src/a.ts").unwrap();
        let auth_pos = files_section.find("src/auth.ts").unwrap();
        let z_pos = files_section.find("src/z.ts").unwrap();
        assert!(a_pos < auth_pos && auth_pos < z_pos, "Files should be sorted");
        let count = files_section.matches("src/auth.ts").count();
        assert_eq!(count, 1, "Duplicate files should be deduped");
    }

    #[test]
    fn handoff_empty_optional_sections_omitted() {
        let mut ctx = make_test_context();
        ctx.tool_activity.clear();
        ctx.reasoning_notes.clear();
        ctx.pending_tasks.clear();
        ctx.files_modified.clear();
        let md = generate_handoff_markdown(&ctx);
        assert!(!md.contains("## Tool Activity"), "Tool Activity should be omitted when empty");
        assert!(!md.contains("## Session Notes"), "Session Notes should be omitted when empty");
        assert!(!md.contains("## Pending Tasks"), "Pending Tasks should be omitted when empty");
        assert!(!md.contains("## Files Modified"), "Files Modified should be omitted when empty");
    }

    #[test]
    fn handoff_closing_directive_present() {
        let ctx = make_test_context();
        let md = generate_handoff_markdown(&ctx);
        assert!(md.contains("---"), "Should have closing separator");
        assert!(
            md.contains("Pick up from where the previous session left off"),
            "Should have closing directive"
        );
    }

    #[test]
    fn handoff_all_source_labels() {
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
        for (source, expected) in sources {
            assert_eq!(source_label(&source), expected);
        }
    }

    #[test]
    fn fmt_commas_works() {
        assert_eq!(fmt_commas(0), "0");
        assert_eq!(fmt_commas(999), "999");
        assert_eq!(fmt_commas(1000), "1,000");
        assert_eq!(fmt_commas(45230), "45,230");
        assert_eq!(fmt_commas(1000000), "1,000,000");
    }
}
