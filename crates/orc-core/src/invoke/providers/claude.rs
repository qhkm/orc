use std::process::Command;
use crate::invoke::provider::{DoctorReport, InvokeOpts, Provider};
use crate::session::types::StreamEvent;

/// Claude provider: uses `claude --output-format stream-json -p <message>`
pub struct ClaudeProvider;

impl Provider for ClaudeProvider {
    fn name(&self) -> &str {
        "claude"
    }

    fn binary(&self) -> &str {
        "claude"
    }

    fn build_command(&self, opts: &InvokeOpts) -> anyhow::Result<Command> {
        let mut cmd = Command::new("claude");
        cmd.arg("--output-format").arg("stream-json");
        cmd.arg("--dangerously-skip-permissions");
        cmd.arg("-p").arg(&opts.message);
        cmd.current_dir(&opts.working_dir);
        Ok(cmd)
    }

    fn parse_stream(&self, line: &str) -> Option<StreamEvent> {
        let line = line.trim();
        if line.is_empty() {
            return None;
        }
        let Ok(val) = serde_json::from_str::<serde_json::Value>(line) else {
            return None;
        };

        let event_type = val.get("type")?.as_str()?;

        match event_type {
            "assistant" => {
                // Extract text from content array
                if let Some(content) = val.get("message").and_then(|m| m.get("content")) {
                    if let Some(arr) = content.as_array() {
                        for item in arr {
                            if item.get("type").and_then(|t| t.as_str()) == Some("text") {
                                if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                                    if !text.is_empty() {
                                        return Some(StreamEvent::Text { content: text.to_string() });
                                    }
                                }
                            }
                        }
                    }
                }
                None
            }
            "result" => Some(StreamEvent::Done),
            "system" => {
                if let Some(msg) = val.get("subtype").and_then(|s| s.as_str()) {
                    Some(StreamEvent::Status { message: msg.to_string() })
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn detect(&self) -> DoctorReport {
        let installed = std::process::Command::new("which")
            .arg("claude")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        DoctorReport {
            name: "claude".to_string(),
            installed,
            binary_path: None,
            auth_ok: installed, // simplified
            error: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_assistant_text_event() {
        let provider = ClaudeProvider;
        let line = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Hello world"}]}}"#;
        let event = provider.parse_stream(line);
        assert!(matches!(event, Some(StreamEvent::Text { content }) if content == "Hello world"));
    }

    #[test]
    fn parse_result_event() {
        let provider = ClaudeProvider;
        let line = r#"{"type":"result","subtype":"success"}"#;
        let event = provider.parse_stream(line);
        assert!(matches!(event, Some(StreamEvent::Done)));
    }

    #[test]
    fn build_command_includes_stream_json() {
        let provider = ClaudeProvider;
        let opts = InvokeOpts {
            message: "test message".into(),
            working_dir: "/tmp".into(),
            ..Default::default()
        };
        let cmd = provider.build_command(&opts).unwrap();
        let args: Vec<_> = cmd.get_args().map(|a| a.to_str().unwrap()).collect();
        assert!(args.contains(&"--output-format"));
        assert!(args.contains(&"stream-json"));
        assert!(args.contains(&"-p"));
        assert!(args.contains(&"test message"));
    }
}
