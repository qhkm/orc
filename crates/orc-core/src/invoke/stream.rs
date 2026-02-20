use anyhow::Result;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command as TokioCommand;
use crate::invoke::provider::{InvokeOpts, Provider};
use crate::session::types::StreamEvent;

/// Result of invoking a provider
#[derive(Debug, Clone)]
pub struct InvokeResult {
    pub response: String,
    pub raw_output: String,
    pub exit_code: i32,
}

/// Invoke a provider and collect all output (non-streaming)
pub async fn execute(provider: &dyn Provider, opts: InvokeOpts) -> Result<InvokeResult> {
    let std_command = provider.build_command(&opts)?;
    let mut command = TokioCommand::from(std_command);
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::piped());

    let mut child = command.spawn()?;
    let stdout = child.stdout.take().unwrap();
    let reader = BufReader::new(stdout);
    let mut lines = reader.lines();

    let mut raw_lines = Vec::new();
    let mut response_parts = Vec::new();

    while let Ok(Some(line)) = lines.next_line().await {
        raw_lines.push(line.clone());
        if let Some(event) = provider.parse_stream(&line) {
            match event {
                StreamEvent::Text { content } => response_parts.push(content),
                StreamEvent::Done => break,
                _ => {}
            }
        }
    }

    let exit_status = child.wait().await?;
    let exit_code = exit_status.code().unwrap_or(-1);

    Ok(InvokeResult {
        response: response_parts.join(""),
        raw_output: raw_lines.join("\n"),
        exit_code,
    })
}

/// Invoke a provider with streaming callback
pub async fn execute_stream<F>(
    provider: &dyn Provider,
    opts: InvokeOpts,
    mut on_event: F,
) -> Result<InvokeResult>
where
    F: FnMut(StreamEvent) + Send,
{
    let std_command = provider.build_command(&opts)?;
    let mut command = TokioCommand::from(std_command);
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::piped());

    let mut child = command.spawn()?;
    let stdout = child.stdout.take().unwrap();
    let reader = BufReader::new(stdout);
    let mut lines = reader.lines();

    let mut raw_lines = Vec::new();
    let mut response_parts = Vec::new();

    while let Ok(Some(line)) = lines.next_line().await {
        raw_lines.push(line.clone());
        if let Some(event) = provider.parse_stream(&line) {
            match &event {
                StreamEvent::Text { content } => response_parts.push(content.clone()),
                _ => {}
            }
            let should_break = matches!(event, StreamEvent::Done);
            on_event(event);
            if should_break {
                break;
            }
        }
    }

    let exit_status = child.wait().await?;
    let exit_code = exit_status.code().unwrap_or(-1);

    Ok(InvokeResult {
        response: response_parts.join(""),
        raw_output: raw_lines.join("\n"),
        exit_code,
    })
}
