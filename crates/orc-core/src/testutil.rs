//! Test-only helpers.

use std::process::Command;
use crate::invoke::provider::{DoctorReport, InvokeOpts, Provider};
use crate::session::types::StreamEvent;

/// A fake provider that runs a shell script instead of an agent CLI.
///
/// Output protocol, one event per stdout line:
/// - `DONE` → `StreamEvent::Done`
/// - `ERROR: <msg>` → `StreamEvent::Error`
/// - anything else → `StreamEvent::Text`
///
/// The prompt is available to the script as `$ORC_MESSAGE`.
pub struct ShellProvider {
    binary: String,
    script: String,
}

impl ShellProvider {
    pub fn new(script: &str) -> Self {
        Self::with_binary("sh", script)
    }

    pub fn with_binary(binary: &str, script: &str) -> Self {
        Self { binary: binary.to_string(), script: script.to_string() }
    }
}

impl Provider for ShellProvider {
    fn name(&self) -> &str {
        "shell"
    }

    fn binary(&self) -> &str {
        &self.binary
    }

    fn build_command(&self, opts: &InvokeOpts) -> anyhow::Result<Command> {
        let mut cmd = Command::new(&self.binary);
        cmd.arg("-c").arg(&self.script);
        cmd.env("ORC_MESSAGE", &opts.message);
        cmd.current_dir(&opts.working_dir);
        Ok(cmd)
    }

    fn parse_stream(&self, line: &str) -> Option<StreamEvent> {
        if line == "DONE" {
            return Some(StreamEvent::Done);
        }
        if let Some(msg) = line.strip_prefix("ERROR: ") {
            return Some(StreamEvent::Error { message: msg.to_string() });
        }
        Some(StreamEvent::Text { content: line.to_string() })
    }

    fn detect(&self) -> DoctorReport {
        DoctorReport { name: "shell".into(), installed: true, binary_path: None, auth_ok: true, error: None }
    }
}
