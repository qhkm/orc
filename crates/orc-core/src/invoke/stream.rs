use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command as TokioCommand;
use crate::invoke::provider::{InvokeOpts, Provider};
use crate::session::types::StreamEvent;

/// How long to wait for a process to exit after it signals Done, and for
/// stderr to drain after the process is gone.
const GRACE: Duration = Duration::from_secs(10);

/// Why the provider process stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExitReason {
    /// The process exited (or signalled Done) on its own.
    Exited,
    /// Killed after running past `timeout_secs`.
    Timeout,
    /// Killed after producing no output for `idle_timeout_secs`.
    Idle,
}

/// Result of invoking a provider
#[derive(Debug, Clone)]
pub struct InvokeResult {
    pub response: String,
    pub raw_output: String,
    pub stderr: String,
    /// Messages from `StreamEvent::Error` events.
    pub errors: Vec<String>,
    /// Process exit code; -1 if killed or terminated by a signal.
    pub exit_code: i32,
    pub exit_reason: ExitReason,
    pub duration_ms: u64,
    /// Reported by the CLI, if it reports cost (see `Provider::extract_cost`).
    pub cost_usd: Option<f64>,
}

/// Invoke a provider and collect all output (non-streaming)
pub async fn execute(provider: &dyn Provider, opts: InvokeOpts) -> Result<InvokeResult> {
    execute_stream(provider, opts, |_| {}).await
}

/// Invoke a provider, calling `on_event` for every parsed event.
///
/// Enforces `opts.timeout_secs` (total) and `opts.idle_timeout_secs` (no stdout),
/// killing the provider's whole process group when either expires.
pub async fn execute_stream<F>(
    provider: &dyn Provider,
    opts: InvokeOpts,
    mut on_event: F,
) -> Result<InvokeResult>
where
    F: FnMut(StreamEvent) + Send,
{
    let started = Instant::now();
    let mut std_command = provider.build_command(&opts)?;
    #[cfg(unix)]
    {
        // Own process group, so a kill also reaches the agent's children.
        use std::os::unix::process::CommandExt;
        std_command.process_group(0);
    }
    let mut command = TokioCommand::from(std_command);
    command.stdin(std::process::Stdio::null());
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::piped());
    command.kill_on_drop(true);

    let mut child = command
        .spawn()
        .with_context(|| format!("failed to start provider '{}'", provider.name()))?;
    let pid = child.id();
    let stdout = child.stdout.take().context("provider stdout not captured")?;
    let mut stderr = child.stderr.take().context("provider stderr not captured")?;

    // Read stderr concurrently into a shared buffer, so a chatty process can't
    // block on a full pipe and we keep partial output if we have to kill it.
    let stderr_buf = Arc::new(Mutex::new(Vec::<u8>::new()));
    let stderr_task = {
        let buf = Arc::clone(&stderr_buf);
        tokio::spawn(async move {
            let mut chunk = [0u8; 4096];
            while let Ok(n) = stderr.read(&mut chunk).await {
                if n == 0 {
                    break;
                }
                buf.lock().unwrap().extend_from_slice(&chunk[..n]);
            }
        })
    };

    let deadline = (opts.timeout_secs > 0).then(|| started + Duration::from_secs(opts.timeout_secs));
    let idle = opts.idle_timeout_secs.filter(|s| *s > 0).map(Duration::from_secs);

    let mut lines = BufReader::new(stdout).lines();
    let mut raw_lines = Vec::new();
    let mut response_parts = Vec::new();
    let mut errors = Vec::new();
    let mut exit_reason = ExitReason::Exited;

    loop {
        let mut wait = idle;
        if let Some(deadline) = deadline {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                exit_reason = ExitReason::Timeout;
                break;
            }
            wait = Some(wait.map_or(left, |w| w.min(left)));
        }

        let next = match wait {
            Some(wait) => match tokio::time::timeout(wait, lines.next_line()).await {
                Ok(next) => next,
                Err(_) => {
                    let past_deadline = deadline.is_some_and(|d| Instant::now() >= d);
                    exit_reason = if past_deadline { ExitReason::Timeout } else { ExitReason::Idle };
                    break;
                }
            },
            None => lines.next_line().await,
        };

        let Ok(Some(line)) = next else { break };
        raw_lines.push(line.clone());
        if let Some(event) = provider.parse_stream(&line) {
            match &event {
                StreamEvent::Text { content } => response_parts.push(content.clone()),
                StreamEvent::Error { message } => {
                    // Some CLIs report one failure twice (codex: `error` then `turn.failed`).
                    if errors.last() == Some(message) {
                        continue;
                    }
                    errors.push(message.clone());
                }
                _ => {}
            }
            let done = matches!(event, StreamEvent::Done);
            on_event(event);
            if done {
                break;
            }
        }
    }

    let exit_code = if exit_reason == ExitReason::Exited {
        match tokio::time::timeout(GRACE, child.wait()).await {
            Ok(status) => status?.code().unwrap_or(-1),
            Err(_) => {
                kill_group(pid, &mut child).await;
                -1
            }
        }
    } else {
        kill_group(pid, &mut child).await;
        -1
    };

    // A killed agent's orphaned children may still hold stderr open; don't wait on them forever.
    let _ = tokio::time::timeout(GRACE, stderr_task).await;
    let stderr = String::from_utf8_lossy(&stderr_buf.lock().unwrap()).into_owned();

    let raw_output = raw_lines.join("\n");
    Ok(InvokeResult {
        response: response_parts.join(""),
        cost_usd: provider.extract_cost(&raw_output),
        raw_output,
        stderr,
        errors,
        exit_code,
        exit_reason,
        duration_ms: started.elapsed().as_millis() as u64,
    })
}

async fn kill_group(pid: Option<u32>, child: &mut tokio::process::Child) {
    #[cfg(unix)]
    if let Some(pid) = pid {
        let _ = std::process::Command::new("kill")
            .arg("-KILL")
            .arg(format!("-{pid}"))
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    let _ = child.kill().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::ShellProvider;

    fn opts(timeout_secs: u64, idle: Option<u64>) -> InvokeOpts {
        InvokeOpts { timeout_secs, idle_timeout_secs: idle, ..Default::default() }
    }

    #[tokio::test]
    async fn collects_text_stderr_and_errors() {
        let p = ShellProvider::new("echo hello; echo ' world'; echo 'ERROR: boom'; echo oops >&2; exit 3");
        let r = execute(&p, opts(30, None)).await.unwrap();
        assert_eq!(r.response, "hello world");
        assert_eq!(r.errors, vec!["boom"]);
        assert!(r.stderr.contains("oops"));
        assert_eq!(r.exit_code, 3);
        assert_eq!(r.exit_reason, ExitReason::Exited);
    }

    #[tokio::test]
    async fn repeated_identical_error_is_reported_once() {
        let p = ShellProvider::new("echo 'ERROR: usage limit'; echo 'ERROR: usage limit'; echo 'ERROR: other'");
        let mut seen = 0;
        let r = execute_stream(&p, opts(30, None), |e| {
            if matches!(e, StreamEvent::Error { .. }) {
                seen += 1;
            }
        })
        .await
        .unwrap();
        assert_eq!(r.errors, vec!["usage limit", "other"]);
        assert_eq!(seen, 2);
    }

    #[tokio::test]
    async fn stops_at_done() {
        let p = ShellProvider::new("echo first; echo DONE; echo after");
        let r = execute(&p, opts(30, None)).await.unwrap();
        assert_eq!(r.response, "first");
        assert_eq!(r.exit_reason, ExitReason::Exited);
    }

    #[tokio::test]
    async fn idle_timeout_kills_silent_process() {
        let p = ShellProvider::new("echo started; echo partial >&2; sleep 30");
        let t = Instant::now();
        let r = execute(&p, opts(60, Some(1))).await.unwrap();
        assert_eq!(r.exit_reason, ExitReason::Idle);
        assert_eq!(r.exit_code, -1);
        assert_eq!(r.response, "started");
        assert!(r.stderr.contains("partial"), "partial stderr kept: {:?}", r.stderr);
        assert!(t.elapsed() < Duration::from_secs(10), "took {:?}", t.elapsed());
    }

    #[tokio::test]
    async fn total_timeout_kills_busy_process() {
        // Prints often enough to never be idle, but runs past the deadline.
        let p = ShellProvider::new("while true; do echo tick; sleep 0.2; done");
        let t = Instant::now();
        let r = execute(&p, opts(1, Some(5))).await.unwrap();
        assert_eq!(r.exit_reason, ExitReason::Timeout);
        assert!(t.elapsed() < Duration::from_secs(10), "took {:?}", t.elapsed());
    }

    #[tokio::test]
    async fn missing_binary_is_an_error() {
        let p = ShellProvider::with_binary("/nonexistent/agent", "");
        assert!(execute(&p, opts(5, None)).await.is_err());
    }
}
