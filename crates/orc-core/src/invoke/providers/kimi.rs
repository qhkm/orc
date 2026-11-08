use std::process::Command;
use crate::invoke::provider::{detect_binary, DoctorReport, InvokeOpts, Provider};
use crate::session::types::StreamEvent;

/// Kimi CLI: `kimi --print --output-format stream-json -p <message>` (kimi-cli 1.26).
///
/// From kimi_cli/ui/print: each stdout line is a message JSON with `role`
/// (`assistant` | `tool`), `content` (a string or a list of `{type: text|think}` parts)
/// and `tool_calls`. **Errors (bad API key, provider errors, max steps) are printed
/// as plain non-JSON text and the process still exits 0**, so non-JSON lines are errors.
pub struct KimiProvider;

impl Provider for KimiProvider {
    fn name(&self) -> &str {
        "kimi"
    }

    fn binary(&self) -> &str {
        "kimi"
    }

    fn build_command(&self, opts: &InvokeOpts) -> anyhow::Result<Command> {
        let mut cmd = Command::new("kimi");
        cmd.arg("--print").arg("--output-format").arg("stream-json");
        cmd.arg("--yolo");
        cmd.arg("--work-dir").arg(&opts.working_dir);
        if let Some(model) = &opts.model {
            cmd.arg("--model").arg(model);
        }
        cmd.arg("--prompt").arg(&opts.message);
        cmd.current_dir(&opts.working_dir);
        Ok(cmd)
    }

    fn parse_stream(&self, line: &str) -> Option<StreamEvent> {
        let line = line.trim();
        if line.is_empty() {
            return None;
        }
        let Ok(val) = serde_json::from_str::<serde_json::Value>(line) else {
            return Some(StreamEvent::Error { message: line.to_string() });
        };
        if val.get("role").and_then(|r| r.as_str()) != Some("assistant") {
            return None;
        }
        if let Some(calls) = val.get("tool_calls").and_then(|c| c.as_array()) {
            if let Some(name) = calls.iter().find_map(|c| c.get("function")?.get("name")?.as_str()) {
                if text_of(&val).is_none() {
                    return Some(StreamEvent::ToolStart { name: name.to_string(), input: String::new() });
                }
            }
        }
        text_of(&val).map(|content| StreamEvent::Text { content })
    }

    fn detect(&self) -> DoctorReport {
        detect_binary("kimi", "kimi")
    }
}

fn text_of(message: &serde_json::Value) -> Option<String> {
    let content = message.get("content")?;
    if let Some(s) = content.as_str() {
        return (!s.is_empty()).then(|| s.to_string());
    }
    let texts: Vec<&str> = content
        .as_array()?
        .iter()
        .filter(|p| p.get("type").and_then(|t| t.as_str()) == Some("text"))
        .filter_map(|p| p.get("text")?.as_str())
        .filter(|t| !t.is_empty())
        .collect();
    (!texts.is_empty()).then(|| texts.join(""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_command() {
        let opts = InvokeOpts { message: "fix it".into(), working_dir: "/tmp/r".into(), ..Default::default() };
        let cmd = KimiProvider.build_command(&opts).unwrap();
        let a: Vec<_> = cmd.get_args().map(|a| a.to_str().unwrap()).collect();
        assert_eq!(a[..3], ["--print", "--output-format", "stream-json"]);
        assert!(a.contains(&"--yolo"));
        assert!(a.windows(2).any(|w| w == ["--work-dir", "/tmp/r"]));
        assert!(a.windows(2).any(|w| w == ["--prompt", "fix it"]));
    }

    #[test]
    fn parses_assistant_messages() {
        let p = KimiProvider;
        let parts = r#"{"role":"assistant","content":[{"type":"think","think":"hmm"},{"type":"text","text":"Fixed "},{"type":"text","text":"add."}]}"#;
        assert!(matches!(p.parse_stream(parts), Some(StreamEvent::Text { content }) if content == "Fixed add."));
        let plain = r#"{"role":"assistant","content":"Done."}"#;
        assert!(matches!(p.parse_stream(plain), Some(StreamEvent::Text { content }) if content == "Done."));
        let call = r#"{"role":"assistant","content":[],"tool_calls":[{"type":"function","id":"c1","function":{"name":"WriteFile","arguments":"{}"}}]}"#;
        assert!(matches!(p.parse_stream(call), Some(StreamEvent::ToolStart { name, .. }) if name == "WriteFile"));
        let tool = r#"{"role":"tool","content":"ok","tool_call_id":"c1"}"#;
        assert!(p.parse_stream(tool).is_none());
    }

    /// kimi-cli 1.26 printed this (exit 0) on 2026-11-08 with an expired key.
    #[test]
    fn plain_text_lines_are_errors() {
        let p = KimiProvider;
        let line = "Error code: 401 - {'error': {'message': 'The API Key appears to be invalid or may have expired.'}}";
        assert!(matches!(p.parse_stream(line), Some(StreamEvent::Error { message }) if message.starts_with("Error code: 401")));
        assert!(p.parse_stream("   ").is_none());
    }
}
