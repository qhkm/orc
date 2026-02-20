use anyhow::Result;
use std::path::PathBuf;
use crate::session::types::SessionSource;

#[derive(Debug, Clone)]
pub struct ResumeCommand {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
}

pub fn build_resume_command(source: SessionSource, session_id: &str, original_path: &str, cwd: &str) -> ResumeCommand {
    match source {
        SessionSource::Claude => ResumeCommand {
            program: "claude".into(),
            args: vec!["--resume".into(), session_id.into()],
            cwd: cwd.into(),
        },
        SessionSource::Codex => ResumeCommand {
            program: "codex".into(),
            args: vec!["-c".into(), format!("experimental_resume={}", original_path)],
            cwd: cwd.into(),
        },
        SessionSource::Gemini => ResumeCommand {
            program: "gemini".into(),
            args: vec!["--continue".into()],
            cwd: cwd.into(),
        },
        SessionSource::Copilot => ResumeCommand {
            program: "copilot".into(),
            args: vec!["--resume".into(), session_id.into()],
            cwd: cwd.into(),
        },
        SessionSource::OpenCode => ResumeCommand {
            program: "opencode".into(),
            args: vec!["--session".into(), session_id.into()],
            cwd: cwd.into(),
        },
        SessionSource::Kimi => ResumeCommand {
            program: "kimi".into(),
            args: vec!["--session".into(), session_id.into()],
            cwd: cwd.into(),
        },
        SessionSource::Amp => ResumeCommand {
            program: "amp".into(),
            args: vec!["--session".into(), session_id.into()],
            cwd: cwd.into(),
        },
        SessionSource::Droid => ResumeCommand {
            program: "droid".into(),
            args: vec!["-s".into(), session_id.into()],
            cwd: cwd.into(),
        },
        SessionSource::Scp => ResumeCommand {
            program: "cat".into(),
            args: vec![original_path.into()],
            cwd: cwd.into(),
        },
    }
}

/// Build a cross-tool command that injects a handoff prompt into the target CLI.
pub fn build_cross_tool_command(target: SessionSource, prompt: &str, cwd: &str) -> ResumeCommand {
    match target {
        SessionSource::Claude => ResumeCommand {
            program: "claude".into(),
            args: vec![prompt.to_string()],
            cwd: cwd.into(),
        },
        SessionSource::Codex => ResumeCommand {
            program: "codex".into(),
            args: vec![prompt.to_string()],
            cwd: cwd.into(),
        },
        SessionSource::Copilot => ResumeCommand {
            program: "copilot".into(),
            args: vec!["-i".into(), prompt.to_string()],
            cwd: cwd.into(),
        },
        SessionSource::Gemini => ResumeCommand {
            program: "gemini".into(),
            args: vec![prompt.to_string()],
            cwd: cwd.into(),
        },
        SessionSource::OpenCode => ResumeCommand {
            program: "opencode".into(),
            args: vec!["--prompt".into(), prompt.to_string()],
            cwd: cwd.into(),
        },
        SessionSource::Kimi => ResumeCommand {
            program: "kimi".into(),
            args: vec![prompt.to_string()],
            cwd: cwd.into(),
        },
        SessionSource::Amp => ResumeCommand {
            program: "amp".into(),
            args: vec![prompt.to_string()],
            cwd: cwd.into(),
        },
        SessionSource::Droid => ResumeCommand {
            program: "droid".into(),
            args: vec!["exec".into(), prompt.to_string()],
            cwd: cwd.into(),
        },
        SessionSource::Scp => ResumeCommand {
            program: "cat".into(),
            args: vec![],
            cwd: cwd.into(),
        },
    }
}

/// Perform cross-tool handoff: write context files, then exec into target CLI.
pub fn cross_tool_resume(ctx: &crate::session::types::SessionContext, target: SessionSource) -> Result<()> {
    use crate::session::handoff::generate_handoff_markdown;
    use crate::session::scp::{write_scp, default_scp_path};

    let cwd = &ctx.session.cwd;

    // Generate handoff markdown
    let mut ctx_with_markdown = ctx.clone();
    ctx_with_markdown.markdown = generate_handoff_markdown(ctx);

    // Write SCP file (non-critical)
    let scp_path = default_scp_path(cwd);
    let _ = write_scp(&scp_path, &ctx_with_markdown);

    // Write handoff markdown to project dir (non-critical)
    let handoff_path = cwd.join(".continues-handoff.md");
    let _ = std::fs::write(&handoff_path, &ctx_with_markdown.markdown);

    // Build prompt — embed full handoff markdown inline
    let intro = format!(
        "I'm continuing a coding session. Here's the full context:\n\n---\n\n{}",
        ctx_with_markdown.markdown
    );

    let cmd = build_cross_tool_command(target, &intro, &cwd.to_string_lossy());
    spawn_and_exit(cmd)
}

/// Replaces current process with the target CLI tool (like exec in shell).
/// Must be called after leaving the TUI alternate screen.
pub fn spawn_and_exit(cmd: ResumeCommand) -> Result<()> {
    use std::os::unix::process::CommandExt;

    let mut command = std::process::Command::new(&cmd.program);
    command.args(&cmd.args);
    command.current_dir(&cmd.cwd);

    // exec() replaces the current process — does not return on success
    let err = command.exec();
    Err(anyhow::anyhow!("exec failed: {}", err))
}

/// Display string for resume command (for UI hints)
pub fn resume_display(cmd: &ResumeCommand) -> String {
    format!("{} {}", cmd.program, cmd.args.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_resume_command() {
        let cmd = build_resume_command(SessionSource::Claude, "abc123", "/path/to/file", "/tmp/project");
        assert_eq!(cmd.program, "claude");
        assert!(cmd.args.contains(&"--resume".to_string()));
        assert!(cmd.args.contains(&"abc123".to_string()));
        assert_eq!(cmd.cwd, PathBuf::from("/tmp/project"));
    }

    #[test]
    fn codex_resume_command_uses_original_path() {
        let cmd = build_resume_command(SessionSource::Codex, "ses123", "/home/.codex/sessions/rollout-abc.jsonl", "/tmp/proj");
        assert_eq!(cmd.program, "codex");
        assert!(cmd.args.iter().any(|a| a.contains("experimental_resume")));
        assert!(cmd.args.iter().any(|a| a.contains("rollout-abc")));
    }

    #[test]
    fn gemini_resume_uses_continue_flag() {
        let cmd = build_resume_command(SessionSource::Gemini, "any", "any", "/tmp/proj");
        assert_eq!(cmd.program, "gemini");
        assert!(cmd.args.contains(&"--continue".to_string()));
    }

    #[test]
    fn all_sources_produce_valid_commands() {
        let sources = [
            SessionSource::Claude, SessionSource::Codex, SessionSource::Gemini,
            SessionSource::Copilot, SessionSource::OpenCode, SessionSource::Kimi,
            SessionSource::Amp, SessionSource::Droid, SessionSource::Scp,
        ];
        for source in sources {
            let cmd = build_resume_command(source, "test-id", "/some/path", "/tmp");
            assert!(!cmd.program.is_empty(), "Program should not be empty for {:?}", source);
        }
    }

    // --- cross-tool tests ---

    #[test]
    fn cross_tool_claude_uses_inline_prompt() {
        let cmd = build_cross_tool_command(SessionSource::Claude, "hello world", "/tmp/proj");
        assert_eq!(cmd.program, "claude");
        assert_eq!(cmd.args, vec!["hello world"]);
        assert_eq!(cmd.cwd, PathBuf::from("/tmp/proj"));
    }

    #[test]
    fn cross_tool_codex_uses_inline_prompt() {
        let cmd = build_cross_tool_command(SessionSource::Codex, "fix the bug", "/tmp/proj");
        assert_eq!(cmd.program, "codex");
        assert_eq!(cmd.args, vec!["fix the bug"]);
    }

    #[test]
    fn cross_tool_copilot_uses_i_flag() {
        let cmd = build_cross_tool_command(SessionSource::Copilot, "continue here", "/tmp/proj");
        assert_eq!(cmd.program, "copilot");
        assert_eq!(cmd.args[0], "-i");
        assert_eq!(cmd.args[1], "continue here");
    }

    #[test]
    fn cross_tool_gemini_uses_inline_prompt() {
        let cmd = build_cross_tool_command(SessionSource::Gemini, "prompt text", "/tmp/proj");
        assert_eq!(cmd.program, "gemini");
        assert_eq!(cmd.args, vec!["prompt text"]);
    }

    #[test]
    fn cross_tool_opencode_uses_prompt_flag() {
        let cmd = build_cross_tool_command(SessionSource::OpenCode, "my prompt", "/tmp/proj");
        assert_eq!(cmd.program, "opencode");
        assert_eq!(cmd.args[0], "--prompt");
        assert_eq!(cmd.args[1], "my prompt");
    }

    #[test]
    fn cross_tool_kimi_uses_inline_prompt() {
        let cmd = build_cross_tool_command(SessionSource::Kimi, "kimi prompt", "/tmp/proj");
        assert_eq!(cmd.program, "kimi");
        assert_eq!(cmd.args, vec!["kimi prompt"]);
    }

    #[test]
    fn cross_tool_amp_uses_inline_prompt() {
        let cmd = build_cross_tool_command(SessionSource::Amp, "amp prompt", "/tmp/proj");
        assert_eq!(cmd.program, "amp");
        assert_eq!(cmd.args, vec!["amp prompt"]);
    }

    #[test]
    fn cross_tool_droid_uses_exec_subcommand() {
        let cmd = build_cross_tool_command(SessionSource::Droid, "droid prompt", "/tmp/proj");
        assert_eq!(cmd.program, "droid");
        assert_eq!(cmd.args[0], "exec");
        assert_eq!(cmd.args[1], "droid prompt");
    }

    #[test]
    fn cross_tool_scp_uses_cat() {
        let cmd = build_cross_tool_command(SessionSource::Scp, "ignored prompt", "/tmp/proj");
        assert_eq!(cmd.program, "cat");
        assert!(cmd.args.is_empty());
    }

    #[test]
    fn all_cross_tool_sources_produce_valid_commands() {
        let sources = [
            SessionSource::Claude, SessionSource::Codex, SessionSource::Gemini,
            SessionSource::Copilot, SessionSource::OpenCode, SessionSource::Kimi,
            SessionSource::Amp, SessionSource::Droid, SessionSource::Scp,
        ];
        for source in sources {
            let cmd = build_cross_tool_command(source, "test prompt", "/tmp");
            assert!(!cmd.program.is_empty(), "Program should not be empty for {:?}", source);
        }
    }

    #[test]
    fn cross_tool_cwd_is_preserved() {
        let sources = [
            SessionSource::Claude, SessionSource::Codex, SessionSource::OpenCode,
            SessionSource::Copilot, SessionSource::Droid,
        ];
        for source in sources {
            let cmd = build_cross_tool_command(source, "prompt", "/custom/cwd");
            assert_eq!(cmd.cwd, PathBuf::from("/custom/cwd"), "CWD should be preserved for {:?}", source);
        }
    }
}
