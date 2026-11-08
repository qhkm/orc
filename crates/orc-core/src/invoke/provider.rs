use anyhow::Result;
use std::process::Command;
use crate::session::types::StreamEvent;

/// Options for invoking a provider
#[derive(Debug, Clone)]
pub struct InvokeOpts {
    pub message: String,
    pub working_dir: String,
    pub model: Option<String>,
    pub continue_session: bool,
    /// Kill the process after this many seconds in total. 0 = no limit.
    pub timeout_secs: u64,
    /// Kill the process if it produces no stdout for this many seconds.
    pub idle_timeout_secs: Option<u64>,
    /// The agent may read the repo but not change it (used for reviewers).
    pub read_only: bool,
}

impl Default for InvokeOpts {
    fn default() -> Self {
        Self {
            message: String::new(),
            working_dir: ".".to_string(),
            model: None,
            continue_session: false,
            timeout_secs: 120,
            idle_timeout_secs: None,
            read_only: false,
        }
    }
}

/// Doctor report for a provider's health status
#[derive(Debug, Clone)]
pub struct DoctorReport {
    pub name: String,
    pub installed: bool,
    pub binary_path: Option<String>,
    pub auth_ok: bool,
    pub error: Option<String>,
}

/// Unified interface for all CLI providers
pub trait Provider: Send + Sync {
    fn name(&self) -> &str;
    fn binary(&self) -> &str;

    /// Build the subprocess command for this provider
    fn build_command(&self, opts: &InvokeOpts) -> Result<Command>;

    /// Parse a single line of output into a StreamEvent.
    /// Returns None if the line doesn't produce a meaningful event.
    fn parse_stream(&self, line: &str) -> Option<StreamEvent>;

    /// Check if this provider is installed and authenticated
    fn detect(&self) -> DoctorReport;

    /// Whether `InvokeOpts::read_only` is enforced by the CLI itself.
    /// Only such providers may act as reviewers.
    fn supports_read_only(&self) -> bool {
        false
    }
}

/// Doctor report for a CLI that is only checked for being on PATH.
pub fn detect_binary(name: &str, binary: &str) -> DoctorReport {
    let path = std::process::Command::new("which")
        .arg(binary)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
    DoctorReport {
        name: name.to_string(),
        installed: path.is_some(),
        binary_path: path.clone(),
        // Sign-in is not checked; `orc run` reports auth failures as a failed attempt.
        auth_ok: path.is_some(),
        error: None,
    }
}
