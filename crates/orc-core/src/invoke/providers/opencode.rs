use std::process::Command;
use crate::invoke::provider::{detect_binary, DoctorReport, InvokeOpts, Provider};
use crate::session::types::StreamEvent;

/// opencode: `opencode run --format json <message>` (verified with opencode 1.18.32).
///
/// Events, one JSON object per line: `step_start`, `text` (`part.text`),
/// `tool_use` (`part.tool`, `part.state.status`), `step_finish` (`part.reason`:
/// `stop` | `tool-calls`), `error` (`error.data.message`). The process exits after the last step.
/// Model names are `provider/model`, so opencode also covers DeepSeek, Qwen, local models, etc.
pub struct OpenCodeProvider;

impl Provider for OpenCodeProvider {
    fn name(&self) -> &str {
        "opencode"
    }

    fn binary(&self) -> &str {
        "opencode"
    }

    fn build_command(&self, opts: &InvokeOpts) -> anyhow::Result<Command> {
        let mut cmd = Command::new("opencode");
        cmd.arg("run").arg("--format").arg("json");
        if opts.read_only {
            // The built-in plan agent cannot edit files or run commands that change them.
            cmd.arg("--agent").arg("plan");
        } else {
            // Without this, permission requests in non-interactive mode are not approved.
            cmd.arg("--auto");
        }
        if let Some(model) = &opts.model {
            cmd.arg("--model").arg(model);
        }
        cmd.arg("--dir").arg(&opts.working_dir);
        cmd.arg(&opts.message);
        cmd.current_dir(&opts.working_dir);
        Ok(cmd)
    }

    fn parse_stream(&self, line: &str) -> Option<StreamEvent> {
        let val: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
        let part = val.get("part");
        match val.get("type")?.as_str()? {
            "text" => {
                let text = part?.get("text")?.as_str()?;
                (!text.is_empty()).then(|| StreamEvent::Text { content: text.to_string() })
            }
            "reasoning" => {
                let text = part?.get("text")?.as_str()?;
                Some(StreamEvent::Reasoning { content: text.to_string() })
            }
            "tool_use" => {
                let name = part?.get("tool").and_then(|t| t.as_str()).unwrap_or("tool");
                Some(StreamEvent::ToolStart { name: name.to_string(), input: String::new() })
            }
            "error" => {
                let error = val.get("error");
                let message = error
                    .and_then(|e| e.get("data"))
                    .and_then(|d| d.get("message"))
                    .and_then(|m| m.as_str())
                    .or_else(|| error.and_then(|e| e.get("name")).and_then(|n| n.as_str()))
                    .unwrap_or("opencode error");
                Some(StreamEvent::Error { message: message.to_string() })
            }
            _ => None,
        }
    }

    fn detect(&self) -> DoctorReport {
        detect_binary("opencode", "opencode")
    }

    fn supports_read_only(&self) -> bool {
        true
    }

    /// Sum of `step_finish.part.cost` over all steps.
    fn extract_cost(&self, raw_output: &str) -> Option<f64> {
        let costs: Vec<f64> = raw_output
            .lines()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .filter(|v| v.get("type").and_then(|t| t.as_str()) == Some("step_finish"))
            .filter_map(|v| v.get("part")?.get("cost")?.as_f64())
            .collect();
        (!costs.is_empty()).then(|| costs.iter().sum())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(opts: &InvokeOpts) -> Vec<String> {
        let cmd = OpenCodeProvider.build_command(opts).unwrap();
        cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect()
    }

    #[test]
    fn build_command() {
        let opts = InvokeOpts {
            message: "fix it".into(),
            working_dir: "/tmp/r".into(),
            model: Some("deepseek/deepseek-chat".into()),
            ..Default::default()
        };
        let a = args(&opts);
        assert_eq!(a[..3], ["run", "--format", "json"]);
        assert!(a.contains(&"--auto".to_string()));
        assert!(a.windows(2).any(|w| w == ["--model", "deepseek/deepseek-chat"]));
        assert!(a.windows(2).any(|w| w == ["--dir", "/tmp/r"]));
        assert_eq!(a.last().unwrap(), "fix it");

        let ro = args(&InvokeOpts { read_only: true, ..Default::default() });
        assert!(ro.windows(2).any(|w| w == ["--agent", "plan"]));
        assert!(!ro.contains(&"--auto".to_string()));
    }

    /// Lines captured from opencode 1.18.32 on 2026-11-08.
    #[test]
    fn parses_real_events() {
        let p = OpenCodeProvider;
        let text = r#"{"type":"text","timestamp":1794148632269,"sessionID":"ses_e4","part":{"id":"prt_1","messageID":"msg_1","sessionID":"ses_e4","type":"text","text":"ok","time":{"start":1,"end":2}}}"#;
        assert!(matches!(p.parse_stream(text), Some(StreamEvent::Text { content }) if content == "ok"));

        let tool = r#"{"type":"tool_use","part":{"type":"tool","tool":"write","state":{"status":"completed","input":{"filePath":"/x/b.txt"}}}}"#;
        assert!(matches!(p.parse_stream(tool), Some(StreamEvent::ToolStart { name, .. }) if name == "write"));

        let err = r#"{"type":"error","error":{"name":"UnknownError","data":{"message":"Unexpected server error. Check server logs for details.","ref":"err_6a9b4e81"}}}"#;
        assert!(matches!(p.parse_stream(err), Some(StreamEvent::Error { message }) if message.starts_with("Unexpected server error")));

        let finish = r#"{"type":"step_finish","part":{"reason":"stop","type":"step-finish","cost":0.0076}}"#;
        assert!(p.parse_stream(finish).is_none());
        assert!(p.parse_stream("not json").is_none());
    }

    #[test]
    fn cost_sums_step_finishes() {
        let raw = "{\"type\":\"step_finish\",\"part\":{\"reason\":\"tool-calls\",\"cost\":0.007681326}}\n{\"type\":\"text\",\"part\":{\"text\":\"done\"}}\n{\"type\":\"step_finish\",\"part\":{\"reason\":\"stop\",\"cost\":0.000166374}}";
        let cost = OpenCodeProvider.extract_cost(raw).unwrap();
        assert!((cost - 0.0078477).abs() < 1e-6, "{cost}");
        assert_eq!(OpenCodeProvider.extract_cost("{\"type\":\"text\"}"), None);
    }
}
