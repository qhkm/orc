use std::collections::HashMap;
use std::io::stdout;
use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::Utc;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use orc_core::config::{
    default_config_path, default_routing, load_config, Config, RouteEntry,
};
use orc_core::session::scp::{default_scp_path, write_scp};
use orc_core::session::types::{
    ConversationMessage, Role, SessionContext, SessionSource, UnifiedSession,
};
use orc_tui::app::App;
use orc_tui::views::embedded::{key_to_bytes, EmbeddedState};
use orc_tui::widgets::PtyScreenWidget;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::{Frame, Terminal};

use crate::capsule::{append_event, capsule_path, CapsuleEvent};
use crate::cost::CostTracker;
use crate::slash::{parse_slash, SlashCommand};

// --- Constants ---

const SLASH_COMMANDS: &[(&str, &str)] = &[
    ("/switch", "switch provider (with context handoff)"),
    ("/clear", "clear terminal scrollback"),
    ("/sessions", "open session hub"),
    ("/providers", "check provider status"),
    ("/help", "list available commands"),
];

const BOX_WIDTH: usize = 42;

// --- REPL state ---

pub struct ReplState {
    pub working_dir: String,
    pub session_id: String,
    pub active_provider: String,
    pub available_providers: Vec<String>,
    pub provider_index: usize,
    pub orc_input: String,
    pub orc_mode: bool,
    pub scroll_offset: u16,
    pub capsule_log_path: PathBuf,
    pub routing: HashMap<String, RouteEntry>,
    pub master_provider: String,
    pub cost_tracker: CostTracker,
    pub started_at: chrono::DateTime<Utc>,
    /// Tracks which providers were used during this session.
    pub provider_history: Vec<String>,
}

impl ReplState {
    pub fn new(working_dir: &str, active_provider: &str, available_providers: Vec<String>) -> Self {
        let started_at = Utc::now();
        let session_id = format!("repl-{}", started_at.format("%Y%m%d-%H%M%S"));
        let capsule_log_path = capsule_path(Path::new(working_dir), &session_id);
        let provider_index = available_providers
            .iter()
            .position(|p| p == active_provider)
            .unwrap_or(0);

        Self {
            working_dir: working_dir.to_string(),
            session_id,
            active_provider: active_provider.to_string(),
            available_providers,
            provider_index,
            orc_input: String::new(),
            orc_mode: false,
            scroll_offset: 0,
            capsule_log_path,
            routing: default_routing(),
            master_provider: "claude".to_string(),
            cost_tracker: CostTracker::new(),
            started_at,
            provider_history: vec![active_provider.to_string()],
        }
    }

    /// Cycle to the next available provider.
    pub fn next_provider(&mut self) {
        if self.available_providers.is_empty() {
            return;
        }
        self.provider_index = (self.provider_index + 1) % self.available_providers.len();
        self.active_provider = self.available_providers[self.provider_index].clone();
        if !self.provider_history.contains(&self.active_provider) {
            self.provider_history.push(self.active_provider.clone());
        }
    }

    pub fn status_bar_text(&self) -> String {
        if self.orc_mode {
            format!("orc> {}", self.orc_input)
        } else {
            format!(
                "{} | \u{21E7}Tab switch | Esc commands",
                self.active_provider
            )
        }
    }
}

// --- Welcome box (shown in first frame before PTY output) ---

pub fn build_welcome_box(version: &str, cwd: &str, provider: &str) -> Vec<Line<'static>> {
    let short_cwd = shorten_path(cwd);
    let top = format!("  \u{256D}{}\u{256E}", "\u{2500}".repeat(BOX_WIDTH));
    let bot = format!("  \u{2570}{}\u{256F}", "\u{2500}".repeat(BOX_WIDTH));

    let title_content = format!("  >_ orc (v{})", version);
    let provider_content = format!("  provider: {}", provider);
    let dir_content = format!("  directory: {}", truncate_str(&short_cwd, 28));

    vec![
        Line::from(Span::styled(top, Style::default().fg(Color::Cyan))),
        Line::from(Span::styled(
            box_line(&title_content),
            Style::default().fg(Color::Cyan),
        )),
        Line::from(Span::styled(
            box_line(""),
            Style::default().fg(Color::Cyan),
        )),
        Line::from(Span::styled(
            box_line(&provider_content),
            Style::default().fg(Color::Cyan),
        )),
        Line::from(Span::styled(
            box_line(&dir_content),
            Style::default().fg(Color::Cyan),
        )),
        Line::from(Span::styled(bot, Style::default().fg(Color::Cyan))),
        Line::from(""),
    ]
}

fn box_line(content: &str) -> String {
    let pad = BOX_WIDTH.saturating_sub(content.len());
    format!(
        "  \u{2502}{}{}\u{2502}",
        content,
        " ".repeat(pad)
    )
}

fn shorten_path(path: &str) -> String {
    if let Some(home) = dirs::home_dir() {
        let home_str = home.to_string_lossy().to_string();
        if path.starts_with(&home_str) {
            return format!("~{}", &path[home_str.len()..]);
        }
    }
    path.to_string()
}

fn truncate_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("...{}", &s[s.len() - (max - 3)..])
    }
}

/// Kept for backward compatibility with tests.
pub fn build_header(version: &str, providers: &[&str], cwd: &str) -> String {
    format!("orc v{} | {} | {}", version, providers.join(" + "), cwd)
}

// --- Context handoff ---

/// Extract visible text from the vt100 screen as a context brief.
///
/// Captures the last N non-empty lines from the terminal screen to pass
/// as context when switching providers.
fn extract_screen_brief(vterm: &vt100::Parser, max_lines: usize) -> String {
    let screen = vterm.screen();
    let cols = screen.size().1;

    // screen.rows(start_col, width) iterates over all visible rows,
    // extracting columns [start_col..start_col+width] from each.
    let all_rows: Vec<String> = screen
        .rows(0, cols)
        .map(|s| s.trim_end().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    // Take the last max_lines non-empty lines.
    let start = all_rows.len().saturating_sub(max_lines);
    all_rows[start..].join("\n")
}

/// Build a handoff prompt that gives the new provider context from the old session.
fn build_handoff_prompt(from_provider: &str, brief: &str, cwd: &str) -> String {
    if brief.is_empty() {
        return String::new();
    }

    format!(
        "I'm continuing work from a {} session in {}. Here's the context from where I left off:\n\n---\n{}\n---\n\nPlease continue from where the previous session left off.",
        from_provider, cwd, brief
    )
}

/// Send a handoff prompt to the PTY after a short delay to let the provider start up.
/// The prompt is typed character by character followed by Enter.
fn send_handoff_to_pty(pty: &mut EmbeddedState, prompt: &str) {
    if prompt.is_empty() {
        return;
    }
    // Send the prompt text then press Enter.
    pty.send_input(prompt.as_bytes());
    pty.send_input(b"\r");
}

// --- Session persistence ---

fn persist_context(state: &ReplState) -> Result<()> {
    let now = Utc::now();

    let summary = format!(
        "orc PTY session with {}",
        state.provider_history.join(", ")
    );

    let session = UnifiedSession {
        id: state.session_id.clone(),
        source: provider_to_source(&state.active_provider),
        cwd: PathBuf::from(&state.working_dir),
        repo: None,
        summary,
        files_modified: vec![],
        updated_at: now,
        created_at: state.started_at,
        message_count: 0,
        original_path: state.capsule_log_path.clone(),
    };

    let context = SessionContext {
        session,
        messages: vec![ConversationMessage {
            role: Role::User,
            content: format!(
                "PTY session using providers: {}",
                state.provider_history.join(", ")
            ),
            model: None,
            timestamp: Some(state.started_at),
        }],
        tool_activity: vec![],
        files_modified: vec![],
        reasoning_notes: vec![],
        pending_tasks: vec![],
        token_usage: None,
        markdown: String::new(),
    };

    let scp_path = default_scp_path(Path::new(&state.working_dir));
    write_scp(&scp_path, &context)
}

fn provider_to_source(provider: &str) -> SessionSource {
    match provider {
        "claude" => SessionSource::Claude,
        "codex" => SessionSource::Codex,
        "gemini" => SessionSource::Gemini,
        "copilot" => SessionSource::Copilot,
        "opencode" => SessionSource::OpenCode,
        "kimi" => SessionSource::Kimi,
        "amp" => SessionSource::Amp,
        "droid" => SessionSource::Droid,
        _ => SessionSource::Scp,
    }
}

// --- Main REPL entry ---

pub fn run_repl() -> Result<()> {
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| ".".to_string());

    let config_path = default_config_path()?;
    let config = if config_path.exists() {
        load_config(&config_path)?
    } else {
        let now = Utc::now();
        Config {
            master_provider: "claude".to_string(),
            providers: vec![],
            routing: Some(default_routing()),
            created_at: now,
            updated_at: now,
        }
    };

    let available_providers = resolve_available_providers(&config);
    let active_provider = config.master_provider.clone();

    let mut state = ReplState::new(&cwd, &active_provider, available_providers);
    state.routing = config.routing.clone().unwrap_or_else(default_routing);
    state.master_provider = config.master_provider.clone();

    enable_raw_mode()?;
    execute!(stdout(), EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;

    // Get terminal size for PTY.
    let term_size = terminal.size()?;
    let pty_rows = term_size.height.saturating_sub(1); // minus status bar
    let pty_cols = term_size.width;

    // Create the vt100 parser matching the PTY area.
    let mut vterm = vt100::Parser::new(pty_rows, pty_cols, 1000);

    // Create the embedded PTY state.
    let mut pty = EmbeddedState::new();

    // Show welcome box in first frame, then start the PTY.
    let welcome_lines = build_welcome_box(env!("CARGO_PKG_VERSION"), &cwd, &state.active_provider);
    let mut show_welcome = true;

    // Determine model from routing (if any).
    let model = state
        .routing
        .values()
        .find(|r| r.provider == state.active_provider)
        .and_then(|r| r.model.clone());

    if let Err(e) = pty.start_provider(
        &state.active_provider,
        &cwd,
        model.as_deref(),
        pty_rows,
        pty_cols,
    ) {
        // If we can't start the provider, show error and exit.
        let _ = disable_raw_mode();
        let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);
        anyhow::bail!("Failed to start {}: {}", state.active_provider, e);
    }

    // Optional: deferred handoff prompt to send after PTY starts producing output.
    let mut pending_handoff: Option<String> = None;

    let loop_result: Result<()> = loop {
        // Defensive: re-enable raw mode in case a subprocess disrupted terminal settings.
        let _ = enable_raw_mode();

        // Drain PTY output and feed to vt100 parser.
        let chunks = pty.poll_raw_output();
        for chunk in &chunks {
            vterm.process(chunk);
            show_welcome = false;
        }

        // If the provider has started producing output and we have a pending handoff, send it.
        if !chunks.is_empty() {
            if let Some(handoff) = pending_handoff.take() {
                // Small delay to let the provider's prompt render.
                std::thread::sleep(std::time::Duration::from_millis(500));
                // Drain any additional output that arrived during the sleep.
                for chunk in pty.poll_raw_output() {
                    vterm.process(&chunk);
                }
                send_handoff_to_pty(&mut pty, &handoff);
            }
        }

        // Render.
        terminal.draw(|frame| {
            render_pty_mode(frame, &state, &vterm, if show_welcome { Some(&welcome_lines) } else { None });
        })?;

        // Poll crossterm events.
        if event::poll(std::time::Duration::from_millis(30))? {
            let evt = event::read()?;
            match evt {
                Event::Key(key) if key.kind == KeyEventKind::Press || key.kind == KeyEventKind::Repeat => {
                    // Ctrl+D: exit orc.
                    if key.code == KeyCode::Char('d')
                        && key.modifiers.contains(KeyModifiers::CONTROL)
                    {
                        break Ok(());
                    }

                    // Esc: toggle orc command mode.
                    if key.code == KeyCode::Esc {
                        state.orc_mode = !state.orc_mode;
                        if !state.orc_mode {
                            state.orc_input.clear();
                        }
                        continue;
                    }

                    // BackTab (Shift+Tab): cycle provider with context handoff.
                    if key.code == KeyCode::BackTab && !state.orc_mode {
                        let old_provider = state.active_provider.clone();
                        let brief = extract_screen_brief(&vterm, 30);

                        state.next_provider();
                        pty.stop();

                        let model = state
                            .routing
                            .values()
                            .find(|r| r.provider == state.active_provider)
                            .and_then(|r| r.model.clone());

                        let size = terminal.size()?;
                        let rows = size.height.saturating_sub(1);
                        let cols = size.width;
                        vterm = vt100::Parser::new(rows, cols, 1000);
                        show_welcome = true;

                        // Prepare context handoff.
                        let handoff = build_handoff_prompt(&old_provider, &brief, &cwd);
                        if !handoff.is_empty() {
                            pending_handoff = Some(handoff);
                        }

                        let _ = append_event(
                            &state.capsule_log_path,
                            &CapsuleEvent::SlashCommand {
                                command: format!("/switch {} (via BackTab from {})", state.active_provider, old_provider),
                                ts: Utc::now(),
                            },
                        );

                        if let Err(e) = pty.start_provider(
                            &state.active_provider,
                            &cwd,
                            model.as_deref(),
                            rows,
                            cols,
                        ) {
                            let msg = format!("Failed to start {}: {}\r\n", state.active_provider, e);
                            vterm.process(msg.as_bytes());
                            show_welcome = false;
                            pending_handoff = None;
                        }
                        continue;
                    }

                    if state.orc_mode {
                        // Orc command mode: buffer keystrokes.
                        match key.code {
                            KeyCode::Enter => {
                                let input = state.orc_input.trim().to_string();
                                state.orc_input.clear();
                                state.orc_mode = false;

                                if !input.is_empty() {
                                    let _ = append_event(
                                        &state.capsule_log_path,
                                        &CapsuleEvent::SlashCommand {
                                            command: input.clone(),
                                            ts: Utc::now(),
                                        },
                                    );

                                    if let Some(cmd) = parse_slash(&input) {
                                        match handle_slash_command(
                                            cmd,
                                            &mut state,
                                            &mut pty,
                                            &mut vterm,
                                            &mut show_welcome,
                                            &mut pending_handoff,
                                            &cwd,
                                            &mut terminal,
                                        ) {
                                            Ok(SlashEffect::Continue) => {}
                                            Ok(SlashEffect::Exit) => break Ok(()),
                                            Err(e) => {
                                                let msg = format!("Error: {}\r\n", e);
                                                vterm.process(msg.as_bytes());
                                            }
                                        }
                                    } else {
                                        let msg = format!("Unknown command: {}\r\n", input);
                                        vterm.process(msg.as_bytes());
                                    }
                                }
                            }
                            KeyCode::Backspace => {
                                state.orc_input.pop();
                            }
                            KeyCode::Char(c) => {
                                if key.modifiers.contains(KeyModifiers::CONTROL) && c == 'u' {
                                    state.orc_input.clear();
                                } else {
                                    state.orc_input.push(c);
                                }
                            }
                            _ => {}
                        }
                    } else {
                        // PTY mode: forward all keystrokes to the PTY.
                        let bytes = key_to_bytes(key.code, key.modifiers);
                        if !bytes.is_empty() {
                            pty.send_input(&bytes);
                        }
                    }
                }
                Event::Resize(cols, rows) => {
                    let pty_rows = rows.saturating_sub(1);
                    vterm = vt100::Parser::new(pty_rows, cols, 1000);
                    pty.resize(pty_rows, cols);
                }
                _ => {}
            }
        }

        // If PTY process exited, show message.
        if !pty.is_running {
            let msg = format!(
                "\r\n[{} exited. Press Esc then /switch <provider>, or Ctrl+D to quit]\r\n",
                state.active_provider
            );
            vterm.process(msg.as_bytes());
            // Drain remaining output.
            for chunk in pty.poll_raw_output() {
                vterm.process(&chunk);
            }

            // Draw one more time to show the exit message.
            terminal.draw(|frame| {
                render_pty_mode(frame, &state, &vterm, None);
            })?;

            // Block until user presses a key.
            loop {
                if let Event::Key(key) = event::read()? {
                    if key.kind != KeyEventKind::Press && key.kind != KeyEventKind::Repeat {
                        continue;
                    }
                    if key.code == KeyCode::Char('d')
                        && key.modifiers.contains(KeyModifiers::CONTROL)
                    {
                        break;
                    }
                    if key.code == KeyCode::Esc {
                        state.orc_mode = true;
                        break;
                    }
                    if key.code == KeyCode::BackTab {
                        let old_provider = state.active_provider.clone();
                        let brief = extract_screen_brief(&vterm, 30);

                        state.next_provider();
                        let size = terminal.size()?;
                        let rows = size.height.saturating_sub(1);
                        let cols = size.width;
                        vterm = vt100::Parser::new(rows, cols, 1000);
                        show_welcome = true;

                        let handoff = build_handoff_prompt(&old_provider, &brief, &cwd);
                        if !handoff.is_empty() {
                            pending_handoff = Some(handoff);
                        }

                        let model = state
                            .routing
                            .values()
                            .find(|r| r.provider == state.active_provider)
                            .and_then(|r| r.model.clone());

                        if let Err(e) = pty.start_provider(
                            &state.active_provider,
                            &cwd,
                            model.as_deref(),
                            rows,
                            cols,
                        ) {
                            let msg = format!("Failed to start {}: {}\r\n", state.active_provider, e);
                            vterm.process(msg.as_bytes());
                            show_welcome = false;
                            pending_handoff = None;
                        }
                        break;
                    }
                }
            }
            if !pty.is_running && !state.orc_mode {
                break Ok(());
            }
        }
    };

    pty.stop();

    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);

    // Persist session context on exit.
    if let Err(e) = persist_context(&state) {
        eprintln!("failed to write .orc/context.json: {}", e);
    }

    if let Err(ref error) = loop_result {
        eprintln!("orc repl exited with error: {}", error);
    }

    loop_result
}

// --- Render: PTY mode ---

fn render_pty_mode(
    frame: &mut Frame<'_>,
    state: &ReplState,
    vterm: &vt100::Parser,
    welcome: Option<&[Line<'_>]>,
) {
    let area = frame.area();

    let chunks = Layout::vertical([
        Constraint::Min(1),    // PTY viewport (vt100 screen)
        Constraint::Length(1), // Status bar
    ])
    .split(area);

    // Main area: PTY screen.
    if let Some(welcome_lines) = welcome {
        let para = Paragraph::new(welcome_lines.to_vec());
        frame.render_widget(para, chunks[0]);
    } else {
        let widget = PtyScreenWidget {
            screen: vterm.screen(),
            scroll_offset: state.scroll_offset,
        };
        frame.render_widget(widget, chunks[0]);
    }

    // Status bar.
    let bar_style = if state.orc_mode {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let bar_text = state.status_bar_text();
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(bar_text, bar_style))),
        chunks[1],
    );

    // Slash completion popup when orc_mode and input starts with '/'.
    if state.orc_mode && state.orc_input.starts_with('/') {
        render_slash_completions(frame, state, chunks[1]);
    }
}

fn render_slash_completions(frame: &mut Frame<'_>, state: &ReplState, status_area: Rect) {
    let prefix = &state.orc_input;
    let matches: Vec<(&str, &str)> = SLASH_COMMANDS
        .iter()
        .filter(|(cmd, _)| cmd.starts_with(prefix))
        .copied()
        .collect();

    if matches.is_empty() {
        return;
    }

    let height = matches.len() as u16;
    let popup_y = status_area.y.saturating_sub(height);
    if popup_y == 0 && status_area.y <= height {
        return;
    }
    let popup_width = status_area.width.min(50);
    let popup_area = Rect::new(status_area.x, popup_y, popup_width, height);

    // Clear background.
    let blank_lines: Vec<Line> = (0..height)
        .map(|_| Line::from(Span::styled(" ".repeat(popup_width as usize), Style::default())))
        .collect();
    frame.render_widget(Paragraph::new(blank_lines), popup_area);

    let lines: Vec<Line> = matches
        .iter()
        .map(|(cmd, desc)| {
            Line::from(vec![
                Span::styled(format!("{:<14}", cmd), Style::default().fg(Color::Cyan)),
                Span::styled(*desc, Style::default().fg(Color::DarkGray)),
            ])
        })
        .collect();

    frame.render_widget(Paragraph::new(lines), popup_area);
}

// --- Slash command handling ---

#[allow(dead_code)]
enum SlashEffect {
    Continue,
    Exit,
}

// Takes the REPL's live pieces individually; grouping them is a larger REPL refactor.
#[allow(clippy::too_many_arguments)]
fn handle_slash_command(
    command: SlashCommand,
    state: &mut ReplState,
    pty: &mut EmbeddedState,
    vterm: &mut vt100::Parser,
    show_welcome: &mut bool,
    pending_handoff: &mut Option<String>,
    cwd: &str,
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
) -> Result<SlashEffect> {
    match command {
        SlashCommand::Help => {
            let mut msg = String::from("\r\nCommands:\r\n");
            for (cmd, desc) in SLASH_COMMANDS {
                msg.push_str(&format!("  {:<14} {}\r\n", cmd, desc));
            }
            msg.push_str("  Esc           toggle command mode\r\n");
            msg.push_str("  Shift+Tab     cycle provider (with context)\r\n");
            msg.push_str("  Ctrl+D        exit orc\r\n");
            vterm.process(msg.as_bytes());
        }
        SlashCommand::Switch { provider } => {
            if !state
                .available_providers
                .iter()
                .any(|p| p == &provider)
            {
                let msg = format!(
                    "Unknown provider: {}. Available: {}\r\n",
                    provider,
                    state.available_providers.join(", ")
                );
                vterm.process(msg.as_bytes());
                return Ok(SlashEffect::Continue);
            }

            let old_provider = state.active_provider.clone();
            let brief = extract_screen_brief(vterm, 30);

            state.active_provider = provider.clone();
            state.provider_index = state
                .available_providers
                .iter()
                .position(|p| p == &provider)
                .unwrap_or(0);
            if !state.provider_history.contains(&state.active_provider) {
                state.provider_history.push(state.active_provider.clone());
            }

            pty.stop();

            let model = state
                .routing
                .values()
                .find(|r| r.provider == state.active_provider)
                .and_then(|r| r.model.clone());

            let size = terminal.size()?;
            let rows = size.height.saturating_sub(1);
            let cols = size.width;
            *vterm = vt100::Parser::new(rows, cols, 1000);
            *show_welcome = true;

            // Prepare context handoff.
            let handoff = build_handoff_prompt(&old_provider, &brief, cwd);
            if !handoff.is_empty() {
                *pending_handoff = Some(handoff);
            }

            if let Err(e) = pty.start_provider(
                &state.active_provider,
                cwd,
                model.as_deref(),
                rows,
                cols,
            ) {
                let msg = format!("Failed to start {}: {}\r\n", state.active_provider, e);
                vterm.process(msg.as_bytes());
                *show_welcome = false;
                *pending_handoff = None;
            }
        }
        SlashCommand::Clear => {
            let size = terminal.size()?;
            let rows = size.height.saturating_sub(1);
            let cols = size.width;
            *vterm = vt100::Parser::new(rows, cols, 1000);
            state.scroll_offset = 0;
        }
        SlashCommand::Sessions => {
            // Temporarily leave alternate screen + raw mode to run the session hub TUI.
            let _ = disable_raw_mode();
            let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);

            let mut app = App::new();
            if let Err(e) = app.run() {
                eprintln!("session hub error: {}", e);
            }

            let _ = enable_raw_mode();
            let _ = execute!(terminal.backend_mut(), EnterAlternateScreen);
            terminal.clear()?;
        }
        SlashCommand::Providers => {
            let mut msg = String::from("\r\nAvailable providers:\r\n");
            for p in &state.available_providers {
                let marker = if p == &state.active_provider {
                    " (active)"
                } else {
                    ""
                };
                msg.push_str(&format!("  {}{}\r\n", p, marker));
            }
            vterm.process(msg.as_bytes());
        }
        // Legacy commands from old REPL — not relevant in PTY mode.
        SlashCommand::Models
        | SlashCommand::Set { .. }
        | SlashCommand::Budget { .. } => {
            vterm.process(b"Command not available in PTY mode. Use /help for available commands.\r\n");
        }
    }

    Ok(SlashEffect::Continue)
}

// --- Helpers ---

fn resolve_available_providers(config: &Config) -> Vec<String> {
    let mut providers = Vec::new();

    for provider in &config.providers {
        if orc_core::invoke::providers::get_provider(&provider.name).is_some()
            && !providers.contains(&provider.name)
        {
            providers.push(provider.name.clone());
        }
    }

    for known in ["claude", "codex"] {
        if !providers.iter().any(|provider| provider == known) {
            providers.push(known.to_string());
        }
    }

    if !providers
        .iter()
        .any(|provider| provider == &config.master_provider)
        && orc_core::invoke::providers::get_provider(&config.master_provider).is_some()
    {
        providers.insert(0, config.master_provider.clone());
    }

    providers
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repl_state_initial() {
        let state = ReplState::new("/tmp/project", "claude", vec!["claude".into(), "codex".into()]);
        assert_eq!(state.working_dir, "/tmp/project");
        assert_eq!(state.active_provider, "claude");
        assert!(!state.orc_mode);
        assert!(state.orc_input.is_empty());
        assert_eq!(state.provider_index, 0);
        assert_eq!(state.provider_history, vec!["claude"]);
    }

    #[test]
    fn repl_state_next_provider() {
        let mut state = ReplState::new(
            "/tmp/project",
            "claude",
            vec!["claude".into(), "codex".into(), "kimi".into()],
        );
        assert_eq!(state.active_provider, "claude");

        state.next_provider();
        assert_eq!(state.active_provider, "codex");
        assert_eq!(state.provider_history, vec!["claude", "codex"]);

        state.next_provider();
        assert_eq!(state.active_provider, "kimi");
        assert_eq!(state.provider_history, vec!["claude", "codex", "kimi"]);

        state.next_provider();
        assert_eq!(state.active_provider, "claude"); // wraps around
        // "claude" already in history, not duplicated
        assert_eq!(state.provider_history, vec!["claude", "codex", "kimi"]);
    }

    #[test]
    fn status_bar_pty_mode() {
        let state = ReplState::new("/tmp/project", "claude", vec!["claude".into()]);
        let bar = state.status_bar_text();
        assert!(bar.contains("claude"));
        assert!(bar.contains("Esc commands"));
    }

    #[test]
    fn status_bar_orc_mode() {
        let mut state = ReplState::new("/tmp/project", "claude", vec!["claude".into()]);
        state.orc_mode = true;
        state.orc_input = "/hel".to_string();
        let bar = state.status_bar_text();
        assert!(bar.contains("orc>"));
        assert!(bar.contains("/hel"));
    }

    #[test]
    fn build_header_includes_providers() {
        let header = build_header("0.1.0", &["claude", "codex"], "/tmp/project");
        assert!(header.contains("claude"));
        assert!(header.contains("codex"));
        assert!(header.contains("0.1.0"));
    }

    #[test]
    fn welcome_box_contains_version() {
        let lines = build_welcome_box("0.1.0", "/tmp/project", "claude");
        let text: String = lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("0.1.0"));
        assert!(text.contains("claude"));
    }

    #[test]
    fn shorten_path_replaces_home() {
        if let Some(home) = dirs::home_dir() {
            let full = format!("{}/projects/test", home.display());
            let short = shorten_path(&full);
            assert!(short.starts_with("~/"));
            assert!(short.contains("projects/test"));
        }
    }

    #[test]
    fn truncate_str_short_unchanged() {
        assert_eq!(truncate_str("hello", 10), "hello");
    }

    #[test]
    fn truncate_str_long_ellipsis() {
        let result = truncate_str("very/long/path/to/project", 15);
        assert!(result.starts_with("..."));
        assert_eq!(result.len(), 15);
    }

    #[test]
    fn extract_screen_brief_captures_text() {
        let mut parser = vt100::Parser::new(24, 80, 0);
        parser.process(b"Line one\r\nLine two\r\nLine three\r\n");
        let brief = extract_screen_brief(&parser, 10);
        assert!(brief.contains("Line one"));
        assert!(brief.contains("Line two"));
        assert!(brief.contains("Line three"));
    }

    #[test]
    fn extract_screen_brief_limits_lines() {
        let mut parser = vt100::Parser::new(24, 80, 0);
        for i in 0..20 {
            parser.process(format!("Line {}\r\n", i).as_bytes());
        }
        let brief = extract_screen_brief(&parser, 5);
        let line_count = brief.lines().count();
        assert!(line_count <= 5, "Expected <= 5 lines, got {}", line_count);
    }

    #[test]
    fn extract_screen_brief_empty_screen() {
        let parser = vt100::Parser::new(24, 80, 0);
        let brief = extract_screen_brief(&parser, 10);
        assert!(brief.is_empty());
    }

    #[test]
    fn build_handoff_prompt_with_brief() {
        let prompt = build_handoff_prompt("claude", "some context", "/tmp/project");
        assert!(prompt.contains("claude"));
        assert!(prompt.contains("some context"));
        assert!(prompt.contains("/tmp/project"));
    }

    #[test]
    fn build_handoff_prompt_empty_brief() {
        let prompt = build_handoff_prompt("claude", "", "/tmp/project");
        assert!(prompt.is_empty());
    }

    #[test]
    fn provider_to_source_mapping() {
        assert_eq!(provider_to_source("claude"), SessionSource::Claude);
        assert_eq!(provider_to_source("codex"), SessionSource::Codex);
        assert_eq!(provider_to_source("unknown"), SessionSource::Scp);
    }
}
