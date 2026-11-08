use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::{Duration, Instant};
use tokio::process::Command;
use super::tail;

/// Result of one `--check` shell command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckResult {
    pub command: String,
    pub passed: bool,
    /// None if the command was killed or could not start.
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    /// Last lines of combined stdout + stderr.
    pub output_tail: String,
    pub duration_ms: u64,
}

/// Run `command` with `sh -c` in `dir`. Exit 0 passes; anything else, a timeout,
/// or failing to start fails.
pub async fn run_check(command: &str, dir: &Path, timeout: Duration) -> CheckResult {
    let started = Instant::now();
    let mut std_cmd = std::process::Command::new("sh");
    std_cmd.arg("-c").arg(command).current_dir(dir);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        std_cmd.process_group(0);
    }
    let mut cmd = Command::from(std_cmd);
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);

    let result = |passed, exit_code, timed_out, output: String| CheckResult {
        command: command.to_string(),
        passed,
        exit_code,
        timed_out,
        output_tail: tail(&output, 40, 4000),
        duration_ms: started.elapsed().as_millis() as u64,
    };

    let child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => return result(false, None, false, format!("failed to start check: {e}")),
    };
    let pid = child.id();

    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(out)) => {
            let mut output = String::from_utf8_lossy(&out.stdout).into_owned();
            let stderr = String::from_utf8_lossy(&out.stderr);
            if !stderr.trim().is_empty() {
                if !output.is_empty() && !output.ends_with('\n') {
                    output.push('\n');
                }
                output.push_str(&stderr);
            }
            let code = out.status.code();
            result(code == Some(0), code, false, output)
        }
        Ok(Err(e)) => result(false, None, false, format!("check failed to run: {e}")),
        Err(_) => {
            // The child was dropped (kill_on_drop); also kill anything it started.
            #[cfg(unix)]
            if let Some(pid) = pid {
                let _ = std::process::Command::new("kill").arg("-KILL").arg(format!("-{pid}")).status();
            }
            result(false, None, true, format!("timed out after {}s", timeout.as_secs()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn passing_and_failing_checks() {
        let dir = tempfile::tempdir().unwrap();
        let ok = run_check("echo fine", dir.path(), Duration::from_secs(10)).await;
        assert!(ok.passed);
        assert_eq!(ok.exit_code, Some(0));
        assert_eq!(ok.output_tail, "fine");

        let bad = run_check("echo out; echo 'assertion failed' >&2; exit 101", dir.path(), Duration::from_secs(10)).await;
        assert!(!bad.passed);
        assert_eq!(bad.exit_code, Some(101));
        assert!(bad.output_tail.contains("out"));
        assert!(bad.output_tail.contains("assertion failed"));
    }

    #[tokio::test]
    async fn runs_in_given_directory() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("marker.txt"), "x").unwrap();
        assert!(run_check("test -f marker.txt", dir.path(), Duration::from_secs(10)).await.passed);
    }

    #[tokio::test]
    async fn timeout_fails_quickly() {
        let dir = tempfile::tempdir().unwrap();
        let t = Instant::now();
        let r = run_check("sleep 30", dir.path(), Duration::from_secs(1)).await;
        assert!(!r.passed);
        assert!(r.timed_out);
        assert!(t.elapsed() < Duration::from_secs(10));
    }
}
