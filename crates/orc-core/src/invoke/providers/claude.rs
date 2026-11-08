use std::process::Command;
use crate::invoke::provider::{DoctorReport, InvokeOpts, Provider};
use crate::session::types::StreamEvent;

/// Parsed metadata from the "result" event in Claude's stream-json output.
#[derive(Debug, Clone, Default)]
pub struct ClaudeResultMeta {
    pub result_text: Option<String>,
    pub cost_usd: Option<f64>,
    pub num_turns: Option<u64>,
    pub session_id: Option<String>,
}

/// Extract result metadata from raw stream-json output lines.
/// Looks for the "result" event and parses its fields.
pub fn extract_result_meta(raw_output: &str) -> ClaudeResultMeta {
    let mut meta = ClaudeResultMeta::default();
    for line in raw_output.lines().rev() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(val) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if val.get("type").and_then(|t| t.as_str()) == Some("result") {
            meta.result_text = val.get("result").and_then(|r| r.as_str()).map(|s| s.to_string());
            meta.cost_usd = val.get("cost_usd").and_then(|c| c.as_f64());
            meta.num_turns = val.get("num_turns").and_then(|n| n.as_u64());
            meta.session_id = val.get("session_id").and_then(|s| s.as_str()).map(|s| s.to_string());
            break;
        }
    }
    meta
}

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
        cmd.arg("--max-turns").arg("25");
        if let Some(model) = &opts.model {
            cmd.arg("--model").arg(model);
        }
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
            // Streaming text deltas (primary source during agentic tool use)
            "content_block_delta" => {
                let delta = val.get("delta")?;
                let delta_type = delta.get("type")?.as_str()?;
                match delta_type {
                    "text_delta" => {
                        let text = delta.get("text")?.as_str()?;
                        if !text.is_empty() {
                            return Some(StreamEvent::Text { content: text.to_string() });
                        }
                    }
                    "thinking_delta" => {
                        let text = delta.get("thinking")?.as_str()?;
                        if !text.is_empty() {
                            return Some(StreamEvent::Reasoning { content: text.to_string() });
                        }
                    }
                    _ => {}
                }
                None
            }
            // Tool use events
            "content_block_start" => {
                let block = val.get("content_block")?;
                if block.get("type")?.as_str()? == "tool_use" {
                    let name = block.get("name").and_then(|n| n.as_str()).unwrap_or("unknown");
                    return Some(StreamEvent::ToolStart {
                        name: name.to_string(),
                        input: String::new(),
                    });
                }
                None
            }
            // Full assistant message snapshot (fallback text source)
            "assistant" => {
                if let Some(content) = val.get("message").and_then(|m| m.get("content")) {
                    if let Some(arr) = content.as_array() {
                        let mut texts = Vec::new();
                        for item in arr {
                            if item.get("type").and_then(|t| t.as_str()) == Some("text") {
                                if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                                    if !text.is_empty() {
                                        texts.push(text.to_string());
                                    }
                                }
                            }
                        }
                        if !texts.is_empty() {
                            return Some(StreamEvent::Text { content: texts.join("\n") });
                        }
                    }
                }
                None
            }
            // Final result — extract the result text as Text, then Done
            "result" => {
                // The result event's "result" field contains the final clean response
                if let Some(result_text) = val.get("result").and_then(|r| r.as_str()) {
                    if !result_text.is_empty() {
                        // We can only return one event, so return Done.
                        // The caller should extract the result text from raw_output.
                    }
                }
                Some(StreamEvent::Done)
            }
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
        assert!(!args.contains(&"--model"));
    }

    #[test]
    fn build_command_passes_model_when_set() {
        let provider = ClaudeProvider;
        let opts = InvokeOpts {
            message: "test".into(),
            working_dir: "/tmp".into(),
            model: Some("opus".into()),
            ..Default::default()
        };
        let cmd = provider.build_command(&opts).unwrap();
        let args: Vec<_> = cmd.get_args().map(|a| a.to_str().unwrap()).collect();
        assert!(args.contains(&"--model"));
        assert!(args.contains(&"opus"));
    }

    #[test]
    fn build_command_includes_max_turns() {
        let provider = ClaudeProvider;
        let opts = InvokeOpts {
            message: "test".into(),
            working_dir: "/tmp".into(),
            ..Default::default()
        };
        let cmd = provider.build_command(&opts).unwrap();
        let args: Vec<_> = cmd.get_args().map(|a| a.to_str().unwrap()).collect();
        assert!(args.contains(&"--max-turns"));
        assert!(args.contains(&"25"));
    }

    #[test]
    fn parse_content_block_delta_text() {
        let provider = ClaudeProvider;
        let line = r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello from delta"}}"#;
        let event = provider.parse_stream(line);
        assert!(matches!(event, Some(StreamEvent::Text { content }) if content == "Hello from delta"));
    }

    #[test]
    fn parse_content_block_delta_thinking() {
        let provider = ClaudeProvider;
        let line = r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"Let me think..."}}"#;
        let event = provider.parse_stream(line);
        assert!(matches!(event, Some(StreamEvent::Reasoning { content }) if content == "Let me think..."));
    }

    #[test]
    fn parse_content_block_start_tool_use() {
        let provider = ClaudeProvider;
        let line = r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_123","name":"Bash","input":{}}}"#;
        let event = provider.parse_stream(line);
        assert!(matches!(event, Some(StreamEvent::ToolStart { name, .. }) if name == "Bash"));
    }

    #[test]
    fn parse_result_with_text() {
        let provider = ClaudeProvider;
        let line = r#"{"type":"result","subtype":"success","result":"Final answer here","cost_usd":0.05,"num_turns":3}"#;
        let event = provider.parse_stream(line);
        assert!(matches!(event, Some(StreamEvent::Done)));
    }

    #[test]
    fn extract_result_meta_from_raw_output() {
        let raw = r#"{"type":"system","subtype":"init"}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Hi"}]}}
{"type":"result","subtype":"success","result":"Final response text","cost_usd":0.042,"num_turns":5,"session_id":"sess_abc"}"#;
        let meta = extract_result_meta(raw);
        assert_eq!(meta.result_text.as_deref(), Some("Final response text"));
        assert!((meta.cost_usd.unwrap() - 0.042).abs() < 0.001);
        assert_eq!(meta.num_turns, Some(5));
        assert_eq!(meta.session_id.as_deref(), Some("sess_abc"));
    }

    #[test]
    fn extract_result_meta_missing_result() {
        let raw = r#"{"type":"system","subtype":"init"}
{"type":"assistant","message":{"role":"assistant","content":[]}}"#;
        let meta = extract_result_meta(raw);
        assert!(meta.result_text.is_none());
        assert!(meta.cost_usd.is_none());
    }
}
