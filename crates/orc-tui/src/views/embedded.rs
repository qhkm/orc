use std::collections::VecDeque;
use std::io::{Read, Write};
use std::sync::mpsc::{self, Receiver, TryRecvError};

use anyhow::Result;
use crossterm::event::{KeyCode, KeyModifiers};
use orc_core::invoke::build_resume_command;
use orc_core::session::types::SessionSource;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};

/// Maximum number of output lines retained in memory.
const MAX_LINES: usize = 1000;

/// Holds the live state of a running embedded provider PTY session.
pub struct EmbeddedState {
    /// Raw output lines accumulated from PTY stdout/stderr.
    pub lines: VecDeque<String>,
    /// Writer that sends bytes to the PTY master (provider stdin).
    pub master_writer: Option<Box<dyn Write + Send>>,
    /// Handle to the child process.
    pub child: Option<Box<dyn portable_pty::Child + Send + Sync>>,
    /// Whether the provider process is still running.
    pub is_running: bool,
    /// Channel receiver — background reader thread sends decoded lines here.
    line_rx: Option<Receiver<String>>,
    /// Sender used to signal the reader thread to shut down.
    shutdown_tx: Option<std::sync::mpsc::SyncSender<()>>,
    /// Handle to the background reader thread so it can be joined on stop.
    thread_handle: Option<std::thread::JoinHandle<()>>,
}

impl EmbeddedState {
    /// Create an empty, stopped `EmbeddedState`.
    pub fn new() -> Self {
        Self {
            lines: VecDeque::new(),
            master_writer: None,
            child: None,
            is_running: false,
            line_rx: None,
            shutdown_tx: None,
            thread_handle: None,
        }
    }

    /// Launch the provider CLI identified by `source` / `id` inside a PTY.
    ///
    /// Spawns a background reader thread that forwards PTY output to an mpsc
    /// channel so that `poll_output` can drain it without blocking the TUI
    /// render loop.
    pub fn start(
        &mut self,
        source: SessionSource,
        id: &str,
        original_path: &str,
        cwd: &str,
    ) -> Result<()> {
        let cmd = build_resume_command(source, id, original_path, cwd);

        let pty_system = native_pty_system();
        let pair = pty_system.openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        // Build the subprocess command.
        let mut builder = CommandBuilder::new(&cmd.program);
        for arg in &cmd.args {
            builder.arg(arg);
        }
        builder.cwd(&cmd.cwd);

        // Spawn child attached to the slave side of the PTY.
        let child = pair.slave.spawn_command(builder)?;
        self.child = Some(child);

        // Writer for sending keystrokes to the PTY master.
        let writer = pair.master.take_writer()?;
        self.master_writer = Some(writer);

        // Reader for receiving PTY output.  We move it into a background thread
        // to avoid blocking the TUI event loop.
        let mut reader = pair.master.try_clone_reader()?;
        let (tx, rx) = mpsc::channel::<String>();

        // Shutdown channel: sending () (or dropping the sender) tells the
        // reader thread to exit cleanly.
        let (shutdown_tx, shutdown_rx) = std::sync::mpsc::sync_channel::<()>(1);

        const MAX_PARTIAL: usize = 65536;

        let handle = std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            let mut partial = String::new();
            loop {
                // Check for shutdown signal before blocking on read.
                if shutdown_rx.try_recv().is_ok() {
                    break;
                }
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        // Append raw bytes, converting leniently to UTF-8.
                        partial.push_str(&String::from_utf8_lossy(&buf[..n]));
                        // Flush complete lines.
                        while let Some(pos) = partial.find('\n') {
                            let line = partial[..pos].to_string();
                            let _ = tx.send(line);
                            partial.drain(..=pos);
                        }
                        // Cap the partial buffer to avoid unbounded growth.
                        if partial.len() > MAX_PARTIAL {
                            let keep = partial.split_off(partial.len() - MAX_PARTIAL / 2);
                            partial = keep;
                        }
                    }
                    Err(_) => break,
                }
            }
            // Send any remaining partial line.
            if !partial.is_empty() {
                let _ = tx.send(partial);
            }
        });

        self.line_rx = Some(rx);
        self.shutdown_tx = Some(shutdown_tx);
        self.thread_handle = Some(handle);
        self.is_running = true;
        Ok(())
    }

    /// Write raw bytes to the PTY master (provider stdin / keypresses).
    pub fn send_input(&mut self, data: &[u8]) {
        if let Some(w) = &mut self.master_writer {
            let ok = w.write_all(data).is_ok();
            let _ = w.flush();
            if !ok {
                self.is_running = false;
            }
        }
    }

    /// Non-blocking drain of the background reader channel into `self.lines`.
    ///
    /// Should be called on every TUI tick (~50 ms).
    pub fn poll_output(&mut self) {
        let Some(rx) = &self.line_rx else { return };

        loop {
            match rx.try_recv() {
                Ok(line) => {
                    self.lines.push_back(line);
                    // Keep memory bounded.
                    while self.lines.len() > MAX_LINES {
                        self.lines.pop_front();
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    // Reader thread finished (process exited).
                    self.is_running = false;
                    self.line_rx = None;
                    break;
                }
            }
        }

        // Also check if the child has exited (non-blocking).
        if self.is_running {
            if let Some(child) = &mut self.child {
                match child.try_wait() {
                    Ok(Some(_)) => self.is_running = false,
                    _ => {}
                }
            }
        }
    }

    /// Kill the child process and clean up resources.
    pub fn stop(&mut self) {
        // Signal the reader thread to exit before killing the child.
        // Dropping the sender also works; sending () is more explicit.
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.try_send(());
        }

        if let Some(child) = &mut self.child {
            let _ = child.kill();
        }
        self.is_running = false;
        self.master_writer = None;
        self.child = None;
        self.line_rx = None;

        // Join the reader thread to reclaim its resources.
        if let Some(handle) = self.thread_handle.take() {
            let _ = handle.join();
        }
    }
}

impl Default for EmbeddedState {
    fn default() -> Self {
        Self::new()
    }
}

/// Convert a crossterm key event into the byte sequence that should be sent to
/// the PTY master (i.e. what a real terminal would transmit to the application).
pub fn key_to_bytes(key: KeyCode, modifiers: KeyModifiers) -> Vec<u8> {
    match key {
        KeyCode::Enter => b"\r".to_vec(),
        KeyCode::Backspace => b"\x7f".to_vec(),
        KeyCode::Esc => b"\x1b".to_vec(),
        KeyCode::Up => b"\x1b[A".to_vec(),
        KeyCode::Down => b"\x1b[B".to_vec(),
        KeyCode::Left => b"\x1b[D".to_vec(),
        KeyCode::Right => b"\x1b[C".to_vec(),
        KeyCode::Tab => b"\t".to_vec(),
        KeyCode::Delete => b"\x1b[3~".to_vec(),
        KeyCode::Home => b"\x1b[H".to_vec(),
        KeyCode::End => b"\x1b[F".to_vec(),
        KeyCode::PageUp => b"\x1b[5~".to_vec(),
        KeyCode::PageDown => b"\x1b[6~".to_vec(),
        KeyCode::Char(c) => {
            if modifiers.contains(KeyModifiers::CONTROL) {
                // CTRL+letter → ASCII control code (0x01–0x1A for a–z)
                let byte = (c as u8) & 0x1f;
                vec![byte]
            } else {
                c.to_string().into_bytes()
            }
        }
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_embedded_state_new() {
        let state = EmbeddedState::new();
        assert!(!state.is_running, "is_running should be false on creation");
        assert!(state.lines.is_empty(), "lines should be empty on creation");
        assert!(state.master_writer.is_none());
        assert!(state.child.is_none());
    }

    #[test]
    fn test_key_to_bytes_backspace() {
        let bytes = key_to_bytes(KeyCode::Backspace, KeyModifiers::NONE);
        assert_eq!(bytes, vec![0x7f]);
    }

    #[test]
    fn test_key_to_bytes_ctrl_u() {
        // CTRL+U = 0x15 (kill line in many CLIs)
        let bytes = key_to_bytes(KeyCode::Char('u'), KeyModifiers::CONTROL);
        assert_eq!(bytes, vec![0x15]);
    }

    #[test]
    fn test_key_to_bytes_regular_char() {
        let bytes = key_to_bytes(KeyCode::Char('a'), KeyModifiers::NONE);
        assert_eq!(bytes, b"a".to_vec());
    }

    #[test]
    fn test_key_to_bytes_esc() {
        let bytes = key_to_bytes(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(bytes, vec![0x1b]);
    }

    #[test]
    fn test_key_to_bytes_arrow_down() {
        let bytes = key_to_bytes(KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(bytes, b"\x1b[B".to_vec());
    }

    #[test]
    fn test_poll_output_does_not_block_when_empty() {
        let mut state = EmbeddedState::new();
        // poll_output with no PTY started should return immediately.
        state.poll_output();
        assert!(state.lines.is_empty());
    }
}
