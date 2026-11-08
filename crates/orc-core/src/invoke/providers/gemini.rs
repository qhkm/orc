use std::process::Command;
use crate::invoke::provider::{detect_binary, DoctorReport, InvokeOpts, Provider};
use crate::session::types::StreamEvent;

/// Gemini CLI: `gemini --yolo --prompt <message>`.
///
/// Gemini CLI 0.1.x has no JSON output, so stdout is the plain-text answer and
/// errors go to stderr with a non-zero exit. `--prompt` is appended to stdin, so
/// stdin must be closed (the runner does). Newer releases add `--output-format`;
/// upgrade with `npm i -g @google/gemini-cli` (or volta) for richer output.
pub struct GeminiProvider;

impl Provider for GeminiProvider {
    fn name(&self) -> &str {
        "gemini"
    }

    fn binary(&self) -> &str {
        "gemini"
    }

    fn build_command(&self, opts: &InvokeOpts) -> anyhow::Result<Command> {
        let mut cmd = Command::new("gemini");
        cmd.arg("--yolo");
        if let Some(model) = &opts.model {
            cmd.arg("--model").arg(model);
        }
        cmd.arg("--prompt").arg(&opts.message);
        cmd.current_dir(&opts.working_dir);
        Ok(cmd)
    }

    fn parse_stream(&self, line: &str) -> Option<StreamEvent> {
        plain_text_line(line)
    }

    fn detect(&self) -> DoctorReport {
        detect_binary("gemini", "gemini")
    }
}

/// For text-output CLIs: every non-empty line is response text (newline kept,
/// so the joined response keeps its line breaks).
pub(crate) fn plain_text_line(line: &str) -> Option<StreamEvent> {
    let line = line.trim_end();
    (!line.trim().is_empty()).then(|| StreamEvent::Text { content: format!("{line}\n") })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_command() {
        let opts = InvokeOpts { message: "fix it".into(), model: Some("gemini-2.5-flash".into()), ..Default::default() };
        let cmd = GeminiProvider.build_command(&opts).unwrap();
        let a: Vec<_> = cmd.get_args().map(|a| a.to_str().unwrap()).collect();
        assert_eq!(a, ["--yolo", "--model", "gemini-2.5-flash", "--prompt", "fix it"]);
    }

    #[test]
    fn text_lines_keep_line_breaks() {
        assert!(matches!(GeminiProvider.parse_stream("Fixed add."), Some(StreamEvent::Text { content }) if content == "Fixed add.\n"));
        assert!(GeminiProvider.parse_stream("   ").is_none());
    }
}
