use std::process::Command;
use crate::invoke::provider::{detect_binary, DoctorReport, InvokeOpts, Provider};
use crate::session::types::StreamEvent;

/// pi (pi_agent_rust): `pi --print --mode json <message>`.
///
/// From `src/agent.rs` (`AgentEvent`, `#[serde(tag = "type")]`): `message_end`
/// carries `message` (`role: assistant`, `content: [{type: text, text}]`,
/// `stopReason`, `errorMessage`), `tool_execution_start` has `toolName`, and
/// `agent_end` has an optional `error`. pi needs a provider configured
/// (`--provider` / API key env vars); without one it exits 1 with
/// "No provider configured" on stderr.
pub struct PiProvider;

impl Provider for PiProvider {
    fn name(&self) -> &str {
        "pi"
    }

    fn binary(&self) -> &str {
        "pi"
    }

    fn build_command(&self, opts: &InvokeOpts) -> anyhow::Result<Command> {
        let mut cmd = Command::new("pi");
        cmd.arg("--print").arg("--mode").arg("json");
        if opts.read_only {
            // The reviewer prompt already contains the diff.
            cmd.arg("--no-tools");
        }
        if let Some(model) = &opts.model {
            cmd.arg("--model").arg(model);
        }
        cmd.arg(&opts.message);
        cmd.current_dir(&opts.working_dir);
        Ok(cmd)
    }

    fn parse_stream(&self, line: &str) -> Option<StreamEvent> {
        let val: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
        match val.get("type")?.as_str()? {
            "message_end" => {
                let message = val.get("message")?;
                if message.get("role")?.as_str()? != "assistant" {
                    return None;
                }
                if message.get("stopReason").and_then(|s| s.as_str()) == Some("error") {
                    let error = message.get("errorMessage").and_then(|e| e.as_str()).unwrap_or("pi request error");
                    return Some(StreamEvent::Error { message: error.to_string() });
                }
                let texts: Vec<&str> = message
                    .get("content")?
                    .as_array()?
                    .iter()
                    .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"))
                    .filter_map(|b| b.get("text")?.as_str())
                    .collect();
                let text = texts.join("");
                (!text.is_empty()).then_some(StreamEvent::Text { content: text })
            }
            "tool_execution_start" => {
                let name = val.get("toolName").and_then(|n| n.as_str()).unwrap_or("tool");
                Some(StreamEvent::ToolStart { name: name.to_string(), input: String::new() })
            }
            "agent_end" => match val.get("error").and_then(|e| e.as_str()) {
                Some(error) => Some(StreamEvent::Error { message: error.to_string() }),
                None => Some(StreamEvent::Done),
            },
            _ => None,
        }
    }

    fn detect(&self) -> DoctorReport {
        detect_binary("pi", "pi")
    }

    fn supports_read_only(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_command() {
        let opts = InvokeOpts { message: "fix it".into(), ..Default::default() };
        let cmd = PiProvider.build_command(&opts).unwrap();
        let a: Vec<_> = cmd.get_args().map(|a| a.to_str().unwrap()).collect();
        assert_eq!(a, ["--print", "--mode", "json", "fix it"]);
        let ro = PiProvider.build_command(&InvokeOpts { read_only: true, ..Default::default() }).unwrap();
        assert!(ro.get_args().any(|a| a == "--no-tools"));
    }

    #[test]
    fn parses_agent_events() {
        let p = PiProvider;
        let text = r#"{"type":"message_end","message":{"role":"assistant","content":[{"type":"thinking","thinking":"x"},{"type":"text","text":"Fixed add."}],"api":"a","provider":"anthropic","model":"m","usage":{},"stopReason":"stop","timestamp":1}}"#;
        assert!(matches!(p.parse_stream(text), Some(StreamEvent::Text { content }) if content == "Fixed add."));
        let user = r#"{"type":"message_end","message":{"role":"user","content":"hi","timestamp":1}}"#;
        assert!(p.parse_stream(user).is_none());
        let failed = r#"{"type":"message_end","message":{"role":"assistant","content":[],"stopReason":"error","errorMessage":"429 rate limit exceeded","timestamp":1}}"#;
        assert!(matches!(p.parse_stream(failed), Some(StreamEvent::Error { message }) if message.contains("rate limit")));
        let tool = r#"{"type":"tool_execution_start","toolCallId":"t1","toolName":"edit","args":{}}"#;
        assert!(matches!(p.parse_stream(tool), Some(StreamEvent::ToolStart { name, .. }) if name == "edit"));
        assert!(matches!(p.parse_stream(r#"{"type":"agent_end","sessionId":"s","messages":[]}"#), Some(StreamEvent::Done)));
        assert!(matches!(p.parse_stream(r#"{"type":"agent_end","sessionId":"s","messages":[],"error":"boom"}"#), Some(StreamEvent::Error { .. })));
    }
}
