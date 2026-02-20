use std::process::Command;
use crate::invoke::provider::{DoctorReport, InvokeOpts, Provider};
use crate::session::types::StreamEvent;

/// Codex provider: uses `codex -q <message>`
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
        // Codex uses -q flag for non-interactive mode
        cmd.arg("-q");
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
            if !line.is_empty() {
                return Some(StreamEvent::Text { content: line.to_string() });
            }
            return None;
        };

        let event_type = val.get("type").and_then(|t| t.as_str())?;

        match event_type {
            "event_msg" => {
                let payload = val.get("payload")?;
                let role = payload.get("role").and_then(|r| r.as_str()).unwrap_or("");
                if role == "assistant" {
                    if let Some(content) = payload.get("content") {
                        if let Some(arr) = content.as_array() {
                            for item in arr {
                                let item_type = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
                                if item_type == "output_text" || item_type == "text" {
                                    if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                                        return Some(StreamEvent::Text { content: text.to_string() });
                                    }
                                }
                            }
                        }
                    }
                }
                None
            }
            _ => None,
        }
    }

    fn detect(&self) -> DoctorReport {
        let installed = std::process::Command::new("which")
            .arg("codex")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        DoctorReport {
            name: "codex".to_string(),
            installed,
            binary_path: None,
            auth_ok: installed,
            error: None,
        }
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
    fn build_codex_command() {
        let provider = CodexProvider;
        let opts = InvokeOpts {
            message: "do something".into(),
            working_dir: "/tmp".into(),
            ..Default::default()
        };
        let cmd = provider.build_command(&opts).unwrap();
        let args: Vec<_> = cmd.get_args().map(|a| a.to_str().unwrap()).collect();
        assert!(args.contains(&"-q"));
        assert!(args.contains(&"do something"));
    }
}
