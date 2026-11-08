use std::process::Command;
use crate::invoke::provider::{DoctorReport, InvokeOpts, Provider};
use crate::session::types::StreamEvent;

/// Codex provider: uses `codex exec --json <message>` (codex-cli 0.1xx).
pub struct CodexProvider;

impl Provider for CodexProvider {
    fn name(&self) -> &str {
        "codex"
    }

    fn binary(&self) -> &str {
        "codex"
    }

    fn build_command(&self, opts: &InvokeOpts) -> anyhow::Result<Command> {
        let mut cmd = Command::new("codex");
        cmd.arg("exec").arg("--json");
        cmd.arg("--skip-git-repo-check");
        // exec defaults to a read-only sandbox; an implementing agent needs to edit the repo.
        let sandbox = if opts.read_only { "read-only" } else { "workspace-write" };
        cmd.arg("--sandbox").arg(sandbox);
        cmd.arg("--cd").arg(&opts.working_dir);
        if let Some(model) = &opts.model {
            cmd.arg("--model").arg(model);
        }
        cmd.arg(&opts.message);
        cmd.current_dir(&opts.working_dir);
        Ok(cmd)
    }

    fn parse_stream(&self, line: &str) -> Option<StreamEvent> {
        let line = line.trim();
        if line.is_empty() {
            return None;
        }
        let Ok(val) = serde_json::from_str::<serde_json::Value>(line) else {
            // Codex may output plain text too
            return Some(StreamEvent::Text { content: line.to_string() });
        };

        let event_type = val.get("type").and_then(|t| t.as_str())?;

        match event_type {
            "item.completed" => {
                let item = val.get("item")?;
                match item.get("type").and_then(|t| t.as_str())? {
                    "agent_message" => {
                        let text = item.get("text")?.as_str()?;
                        Some(StreamEvent::Text { content: text.to_string() })
                    }
                    "reasoning" => {
                        let text = item.get("text")?.as_str()?;
                        Some(StreamEvent::Reasoning { content: text.to_string() })
                    }
                    "command_execution" => Some(StreamEvent::ToolEnd {
                        name: "shell".to_string(),
                        output: item
                            .get("command")
                            .and_then(|c| c.as_str())
                            .unwrap_or_default()
                            .to_string(),
                    }),
                    _ => None,
                }
            }
            "error" => {
                let message = val.get("message").and_then(|m| m.as_str()).unwrap_or("codex error");
                Some(StreamEvent::Error { message: message.to_string() })
            }
            "turn.failed" => {
                let message = val
                    .get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(|m| m.as_str())
                    .unwrap_or("codex turn failed");
                Some(StreamEvent::Error { message: message.to_string() })
            }
            "turn.completed" => Some(StreamEvent::Done),
            // Older session-log format, kept for compatibility.
            "event_msg" => {
                let payload = val.get("payload")?;
                if payload.get("role").and_then(|r| r.as_str()) != Some("assistant") {
                    return None;
                }
                for item in payload.get("content")?.as_array()? {
                    let item_type = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
                    if item_type == "output_text" || item_type == "text" {
                        if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                            return Some(StreamEvent::Text { content: text.to_string() });
                        }
                    }
                }
                None
            }
            _ => None,
        }
    }

    fn supports_read_only(&self) -> bool {
        true
    }

    fn detect(&self) -> DoctorReport {
        crate::invoke::provider::detect_binary("codex", "codex")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_codex_event_msg() {
        let provider = CodexProvider;
        let line = r#"{"timestamp":"2026-02-20T10:00:00Z","type":"event_msg","payload":{"role":"assistant","content":[{"type":"output_text","text":"Done."}]}}"#;
        let event = provider.parse_stream(line);
        assert!(matches!(event, Some(StreamEvent::Text { content }) if content == "Done."));
    }

    #[test]
    fn parse_exec_agent_message_and_turn_completed() {
        let p = CodexProvider;
        let line = r#"{"type":"item.completed","item":{"id":"item_1","type":"agent_message","text":"ok"}}"#;
        assert!(matches!(p.parse_stream(line), Some(StreamEvent::Text { content }) if content == "ok"));
        let line = r#"{"type":"turn.completed","usage":{"input_tokens":10,"output_tokens":2}}"#;
        assert!(matches!(p.parse_stream(line), Some(StreamEvent::Done)));
        let line = r#"{"type":"thread.started","thread_id":"01a1bb5f"}"#;
        assert!(p.parse_stream(line).is_none());
    }

    /// Real output from codex-cli 0.166.0 on 2026-11-08 when the account hit its limit.
    #[test]
    fn parse_exec_usage_limit_errors() {
        let p = CodexProvider;
        let line = r#"{"type":"error","message":"You’ve hit your usage limit. Upgrade to Pro (https://chatgpt.com/explore/pro), visit https://chatgpt.com/settings/usage to purchase more credits or try again at Nov 14th, 2026 3:22 PM."}"#;
        assert!(matches!(p.parse_stream(line), Some(StreamEvent::Error { message }) if message.contains("usage limit")));
        let line = r#"{"type":"turn.failed","error":{"message":"You’ve hit your usage limit."}}"#;
        assert!(matches!(p.parse_stream(line), Some(StreamEvent::Error { message }) if message.contains("usage limit")));
    }

    #[test]
    fn build_codex_command() {
        let provider = CodexProvider;
        let opts = InvokeOpts {
            message: "do something".into(),
            working_dir: "/tmp".into(),
            model: Some("gpt-5".into()),
            ..Default::default()
        };
        let cmd = provider.build_command(&opts).unwrap();
        let args: Vec<_> = cmd.get_args().map(|a| a.to_str().unwrap()).collect();
        assert_eq!(args[..2], ["exec", "--json"]);
        assert!(!args.contains(&"-q"));
        assert!(args.windows(2).any(|w| w == ["--sandbox", "workspace-write"]));
        assert!(args.windows(2).any(|w| w == ["--model", "gpt-5"]));
        assert_eq!(args.last(), Some(&"do something"));
    }
}
