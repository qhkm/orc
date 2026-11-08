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
}
