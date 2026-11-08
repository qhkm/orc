use std::process::Command;
use crate::invoke::provider::{detect_binary, DoctorReport, InvokeOpts, Provider};
use crate::session::types::StreamEvent;
use super::gemini::plain_text_line;

/// GitHub Copilot CLI: `copilot --prompt <message> --silent --allow-all-tools`.
///
/// `--silent` prints only the agent's response as plain text; failures go to
/// stderr with exit 1 (e.g. `CAPIError: 400 The requested model is not supported`
/// when the default model is not on the account's plan; pass a model with `--model`).
pub struct CopilotProvider;

impl Provider for CopilotProvider {
    fn name(&self) -> &str {
        "copilot"
    }

    fn binary(&self) -> &str {
        "copilot"
    }

    fn build_command(&self, opts: &InvokeOpts) -> anyhow::Result<Command> {
        let mut cmd = Command::new("copilot");
        cmd.arg("--prompt").arg(&opts.message);
        cmd.arg("--silent").arg("--no-color");
        cmd.arg("--allow-all-tools");
        if let Some(model) = &opts.model {
            cmd.arg("--model").arg(model);
        }
        cmd.current_dir(&opts.working_dir);
        Ok(cmd)
    }

    fn parse_stream(&self, line: &str) -> Option<StreamEvent> {
        plain_text_line(line)
    }

    fn detect(&self) -> DoctorReport {
        detect_binary("copilot", "copilot")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_command() {
        let opts = InvokeOpts { message: "fix it".into(), model: Some("claude-haiku-4.5".into()), ..Default::default() };
        let cmd = CopilotProvider.build_command(&opts).unwrap();
        let a: Vec<_> = cmd.get_args().map(|a| a.to_str().unwrap()).collect();
        assert!(a.windows(2).any(|w| w == ["--prompt", "fix it"]));
        assert!(a.contains(&"--silent"));
        assert!(a.contains(&"--allow-all-tools"));
        assert!(a.windows(2).any(|w| w == ["--model", "claude-haiku-4.5"]));
    }

    #[test]
    fn plain_text_response() {
        assert!(matches!(CopilotProvider.parse_stream("Done."), Some(StreamEvent::Text { content }) if content == "Done.\n"));
    }
}
