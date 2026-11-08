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
    /// Channel receiver — background reader thread sends raw byte chunks here (PTY mode).
    byte_rx: Option<Receiver<Vec<u8>>>,
    /// Handle to the PTY master for resizing.
    master_pty: Option<Box<dyn portable_pty::MasterPty + Send>>,
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
            byte_rx: None,
            master_pty: None,
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
        let reader = pair.master.try_clone_reader()?;

        self.setup_line_reader_thread(reader);
        self.is_running = true;
        Ok(())
    }

    /// Launch a provider CLI by name (e.g. "claude", "codex") inside a PTY.
    ///
    /// Unlike `start()` which resumes an existing session, this starts a fresh
    /// interactive provider process. Raw byte output is forwarded via `poll_raw_output()`.
    pub fn start_provider(
        &mut self,
        provider: &str,
        cwd: &str,
        model: Option<&str>,
        rows: u16,
        cols: u16,
    ) -> Result<()> {
        // Stop any existing process first.
        if self.is_running {
            self.stop();
        }

        let (program, args) = build_provider_command(provider, model);

        let pty_system = native_pty_system();
        let pair = pty_system.openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        let mut builder = CommandBuilder::new(&program);
        for arg in &args {
            builder.arg(arg);
        }
        builder.cwd(cwd);

        let child = pair.slave.spawn_command(builder)?;
        self.child = Some(child);

        let writer = pair.master.take_writer()?;
        self.master_writer = Some(writer);

        let reader = pair.master.try_clone_reader()?;

        // Store master PTY handle for resizing.
        self.master_pty = Some(pair.master);

        self.setup_byte_reader_thread(reader);
        self.is_running = true;
        Ok(())
    }

    /// Set up a background thread that reads lines (for legacy `start()` mode).
    fn setup_line_reader_thread(&mut self, mut reader: Box<dyn Read + Send>) {
        let (tx, rx) = mpsc::channel::<String>();
        let (shutdown_tx, shutdown_rx) = std::sync::mpsc::sync_channel::<()>(1);

        const MAX_PARTIAL: usize = 65536;

        let handle = std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            let mut partial = String::new();
            loop {
                if shutdown_rx.try_recv().is_ok() {
                    break;
                }
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        partial.push_str(&String::from_utf8_lossy(&buf[..n]));
                        while let Some(pos) = partial.find('\n') {
                            let line = partial[..pos].to_string();
                            let _ = tx.send(line);
                            partial.drain(..=pos);
                        }
                        if partial.len() > MAX_PARTIAL {
                            let keep = partial.split_off(partial.len() - MAX_PARTIAL / 2);
                            partial = keep;
                        }
                    }
                    Err(_) => break,
                }
            }
            if !partial.is_empty() {
                let _ = tx.send(partial);
            }
        });

        self.line_rx = Some(rx);
        self.shutdown_tx = Some(shutdown_tx);
        self.thread_handle = Some(handle);
    }

    /// Set up a background thread that reads raw byte chunks (for PTY mode).
    fn setup_byte_reader_thread(&mut self, mut reader: Box<dyn Read + Send>) {
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        let (shutdown_tx, shutdown_rx) = std::sync::mpsc::sync_channel::<()>(1);

        let handle = std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                if shutdown_rx.try_recv().is_ok() {
                    break;
                }
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if tx.send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        self.byte_rx = Some(rx);
        self.shutdown_tx = Some(shutdown_tx);
        self.thread_handle = Some(handle);
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

    /// Non-blocking drain of raw byte chunks from the PTY reader.
    ///
    /// Returns all pending byte chunks. Feed these directly to a `vt100::Parser`.
    pub fn poll_raw_output(&mut self) -> Vec<Vec<u8>> {
        let mut chunks = Vec::new();
        let Some(rx) = &self.byte_rx else {
            return chunks;
        };

        loop {
            match rx.try_recv() {
                Ok(chunk) => chunks.push(chunk),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.is_running = false;
                    self.byte_rx = None;
                    break;
                }
            }
        }

        // Check if the child has exited.
        if self.is_running {
            if let Some(child) = &mut self.child {
                if let Ok(Some(_)) = child.try_wait() {
                    self.is_running = false;
                }
            }
        }

        chunks
    }

    /// Resize the PTY to new dimensions.
    pub fn resize(&self, rows: u16, cols: u16) {
        if let Some(ref pty) = self.master_pty {
            let _ = pty.resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            });
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
        self.byte_rx = None;
        self.master_pty = None;

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

/// Build the command (program + args) for a given provider.
pub fn build_provider_command(provider: &str, model: Option<&str>) -> (String, Vec<String>) {
    match provider {
        "claude" => {
            let mut args = vec!["--dangerously-skip-permissions".to_string()];
            if let Some(m) = model {
                args.push("--model".to_string());
                args.push(m.to_string());
            }
            ("claude".to_string(), args)
        }
        "codex" => ("codex".to_string(), vec![]),
        other => (other.to_string(), vec![]),
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

    #[test]
    fn test_poll_raw_output_returns_empty_when_no_pty() {
        let mut state = EmbeddedState::new();
        let chunks = state.poll_raw_output();
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_build_provider_command_claude() {
        let (program, args) = build_provider_command("claude", None);
        assert_eq!(program, "claude");
        assert_eq!(args, vec!["--dangerously-skip-permissions"]);
    }

    #[test]
    fn test_build_provider_command_claude_with_model() {
        let (program, args) = build_provider_command("claude", Some("opus"));
        assert_eq!(program, "claude");
        assert_eq!(
            args,
            vec!["--dangerously-skip-permissions", "--model", "opus"]
        );
    }

    #[test]
    fn test_build_provider_command_codex() {
        let (program, args) = build_provider_command("codex", None);
        assert_eq!(program, "codex");
        assert!(args.is_empty());
    }

    #[test]
    fn test_build_provider_command_other() {
        let (program, args) = build_provider_command("kimi", None);
        assert_eq!(program, "kimi");
        assert!(args.is_empty());
    }

    #[test]
    fn test_resize_no_panic_without_pty() {
        let state = EmbeddedState::new();
        // Should not panic when no master_pty is set.
        state.resize(40, 120);
    }
}
