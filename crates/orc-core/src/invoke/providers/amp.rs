use std::process::Command;
use crate::invoke::provider::{detect_binary, DoctorReport, InvokeOpts, Provider};
use crate::session::types::StreamEvent;
use super::ClaudeProvider;

/// Amp: `amp -x <message> --stream-json`.
///
/// `--stream-json` is Claude Code compatible (`system`, `assistant`, `user`,
/// `result` events), so parsing is shared with the Claude provider. A failed run
/// ends with `{"type":"result","subtype":"error_during_execution","is_error":true,"error":…}`
/// (confirmed in the amp 2026-03 bundle). `InvokeOpts::model` is passed as amp's `--mode`.
pub struct AmpProvider;

impl Provider for AmpProvider {
    fn name(&self) -> &str {
        "amp"
    }

    fn binary(&self) -> &str {
        "amp"
    }

    fn build_command(&self, opts: &InvokeOpts) -> anyhow::Result<Command> {
        let mut cmd = Command::new("amp");
        cmd.arg("--execute").arg(&opts.message);
        cmd.arg("--stream-json");
        cmd.arg("--dangerously-allow-all");
        cmd.arg("--no-notifications");
        if let Some(mode) = &opts.model {
            cmd.arg("--mode").arg(mode);
        }
        cmd.current_dir(&opts.working_dir);
        Ok(cmd)
    }

    fn parse_stream(&self, line: &str) -> Option<StreamEvent> {
        ClaudeProvider.parse_stream(line)
    }

    fn detect(&self) -> DoctorReport {
        detect_binary("amp", "amp")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_command() {
        let opts = InvokeOpts { message: "fix it".into(), model: Some("rush".into()), ..Default::default() };
        let cmd = AmpProvider.build_command(&opts).unwrap();
        let a: Vec<_> = cmd.get_args().map(|a| a.to_str().unwrap()).collect();
        assert!(a.windows(2).any(|w| w == ["--execute", "fix it"]));
        assert!(a.contains(&"--stream-json"));
        assert!(a.contains(&"--dangerously-allow-all"));
        assert!(a.windows(2).any(|w| w == ["--mode", "rush"]));
    }

    #[test]
    fn parses_claude_compatible_events_and_amp_errors() {
        let p = AmpProvider;
        let text = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Fixed."}]}}"#;
        assert!(matches!(p.parse_stream(text), Some(StreamEvent::Text { content }) if content == "Fixed."));
        let ok = r#"{"type":"result","subtype":"success","duration_ms":10,"is_error":false,"num_turns":2,"result":"Fixed.","session_id":"T-1"}"#;
        assert!(matches!(p.parse_stream(ok), Some(StreamEvent::Done)));
        let err = r#"{"type":"result","subtype":"error_during_execution","duration_ms":10,"is_error":true,"num_turns":1,"error":"Network timeout","session_id":"T-1"}"#;
        assert!(matches!(p.parse_stream(err), Some(StreamEvent::Error { message }) if message == "Network timeout"));
    }
}
