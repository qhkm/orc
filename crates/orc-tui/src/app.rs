use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::execute;
use orc_core::invoke::{build_resume_command, cross_tool_resume, spawn_and_exit};
use orc_core::session::index::{
    build_index_full, default_index_path, is_stale, load_index, save_index, SessionIndex,
};
use orc_core::session::types::{SessionContext, SessionSource, UnifiedSession};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};
use ratatui::Terminal;
use std::io::{self, stdout};
use std::path::PathBuf;
use crate::views::embedded::{key_to_bytes, EmbeddedState};
use crate::views::flow_runner::{FlowRunnerState, render_flow_runner};
use crate::widgets::pty_viewport::PtyViewportWidget;
use crate::widgets::{AppMode, FooterWidget, SessionListState, SessionListWidget};

/// All session sources in a fixed display order.
const ALL_SOURCES: &[SessionSource] = &[
    SessionSource::Claude,
    SessionSource::Codex,
    SessionSource::Gemini,
    SessionSource::Copilot,
    SessionSource::OpenCode,
    SessionSource::Kimi,
    SessionSource::Amp,
    SessionSource::Droid,
    SessionSource::Scp,
];

fn source_display_name(source: SessionSource) -> &'static str {
    match source {
        SessionSource::Claude => "claude",
        SessionSource::Codex => "codex",
        SessionSource::Gemini => "gemini",
        SessionSource::Copilot => "copilot",
        SessionSource::OpenCode => "opencode",
        SessionSource::Kimi => "kimi",
        SessionSource::Amp => "amp",
        SessionSource::Droid => "droid",
        SessionSource::Scp => "scp",
    }
}

/// Build a minimal SessionContext from a UnifiedSession (no full message history).
fn make_minimal_context(session: &UnifiedSession) -> SessionContext {
    SessionContext {
        session: session.clone(),
        messages: vec![],
        tool_activity: vec![],
        files_modified: vec![],
        reasoning_notes: vec![],
        pending_tasks: vec![],
        token_usage: None,
        markdown: String::new(),
    }
}

/// Compute a centered Rect of fixed width and height inside `area`.
fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect {
        x,
        y,
        width: width.min(area.width),
        height: height.min(area.height),
    }
}

/// State for the "what next?" popup that appears after pressing Enter on a session.
pub struct NextPopupState {
    pub session: UnifiedSession,
    pub options: Vec<SessionSource>,
    pub selected: usize,
}

/// Returns resume options: [source itself, top 2 other common tools].
/// The session's own source is always first; then up to 2 more common tools.
pub fn next_popup_options(source: SessionSource) -> Vec<SessionSource> {
    const COMMON: &[SessionSource] = &[
        SessionSource::Claude,
        SessionSource::Codex,
        SessionSource::Gemini,
        SessionSource::Copilot,
        SessionSource::OpenCode,
    ];
    let mut opts = vec![source];
    for &s in COMMON {
        if s != source && opts.len() < 3 {
            opts.push(s);
        }
    }
    opts
}

pub enum View {
    SessionHub,
    FlowRunner,
    Embedded,
}

pub struct App {
    pub should_quit: bool,
    pub sessions: Vec<UnifiedSession>,
    pub list_state: SessionListState,
    pub mode: AppMode,
    pub search_query: String,
    pub filter: Option<SessionSource>,

    // Cross-tool resume popup state (triggered by 'r' key)
    pub cross_tool_popup: bool,
    pub cross_tool_targets: Vec<SessionSource>,
    pub cross_tool_selected: usize,

    // "What next?" popup state (triggered by Enter key)
    pub next_popup: Option<NextPopupState>,

    // View state
    pub view: View,
    pub flow_runner: FlowRunnerState,
    pub embedded: Option<EmbeddedState>,

    // Current working directory for local session highlighting
    pub project_cwd: Option<PathBuf>,

    // Status message shown in footer after actions like refresh
    pub status_msg: String,
}

impl App {
    pub fn new() -> Self {
        // Load sessions synchronously using a temporary single-threaded runtime.
        // main.rs is not async, so there is no outer runtime — this is safe.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to build tokio runtime");
        let sessions = rt.block_on(async {
            // Try to load a fresh cached index first.
            if let Ok(index_path) = default_index_path() {
                if let Ok(index) = load_index(&index_path) {
                    if !is_stale(&index, 300) {
                        return index.sessions;
                    }
                }
            }
            // Cache is missing or stale — rebuild from all providers.
            let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
            let claude_root = home.join(".claude").join("projects");
            let codex_root = home.join(".codex").join("sessions");
            let index = build_index_full(&claude_root, &codex_root, None, None, None)
                .await
                .unwrap_or_else(|_| SessionIndex {
                    sessions: vec![],
                    built_at: chrono::Utc::now(),
                });
            // Persist for next startup.
            if let Ok(index_path) = default_index_path() {
                let _ = save_index(&index_path, &index);
            }
            index.sessions
        });

        let mut list_state = SessionListState::new();
        if sessions.is_empty() {
            list_state.select(None);
        }

        Self {
            should_quit: false,
            sessions,
            list_state,
            mode: AppMode::Normal,
            search_query: String::new(),
            filter: None,
            cross_tool_popup: false,
            cross_tool_targets: vec![],
            cross_tool_selected: 0,
            next_popup: None,
            view: View::SessionHub,
            flow_runner: FlowRunnerState::new(),
            embedded: None,
            project_cwd: std::env::current_dir().ok(),
            status_msg: String::new(),
        }
    }

    pub fn run(&mut self) -> Result<()> {
        enable_raw_mode()?;
        execute!(stdout(), EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout());
        let mut terminal = Terminal::new(backend)?;

        let result = self.event_loop(&mut terminal);

        disable_raw_mode()?;
        execute!(stdout(), LeaveAlternateScreen)?;

        result
    }

    fn event_loop(&mut self, terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
        loop {
            terminal.draw(|frame| self.render(frame))?;

            if event::poll(std::time::Duration::from_millis(50))? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press {
                        match self.view {
                            View::Embedded => {
                                match key.code {
                                    KeyCode::Esc => {
                                        if let Some(ref mut emb) = self.embedded {
                                            emb.stop();
                                        }
                                        self.embedded = None;
                                        self.view = View::SessionHub;
                                    }
                                    other => {
                                        let bytes = key_to_bytes(other, key.modifiers);
                                        if !bytes.is_empty() {
                                            if let Some(ref mut emb) = self.embedded {
                                                emb.send_input(&bytes);
                                            }
                                        }
                                    }
                                }
                            }
                            View::FlowRunner => {
                                match key.code {
                                    KeyCode::Esc => {
                                        self.view = View::SessionHub;
                                    }
                                    KeyCode::Up | KeyCode::Char('k') => {
                                        self.flow_runner.transcript.scroll_up(1);
                                    }
                                    KeyCode::Down | KeyCode::Char('j') => {
                                        self.flow_runner.transcript.scroll_down(1);
                                    }
                                    _ => {}
                                }
                            }
                            View::SessionHub => {
                                // next_popup takes highest priority
                                if self.next_popup.is_some() {
                                    match key.code {
                                        KeyCode::Esc => {
                                            self.next_popup = None;
                                        }
                                        KeyCode::Up | KeyCode::Char('k') => {
                                            if let Some(popup) = &mut self.next_popup {
                                                if popup.selected > 0 {
                                                    popup.selected -= 1;
                                                }
                                            }
                                        }
                                        KeyCode::Down | KeyCode::Char('j') => {
                                            if let Some(popup) = &mut self.next_popup {
                                                let max = popup.options.len().saturating_sub(1);
                                                if popup.selected < max {
                                                    popup.selected += 1;
                                                }
                                            }
                                        }
                                        KeyCode::Char(c) if matches!(c, '1' | '2' | '3') => {
                                            let idx = (c as u8 - b'1') as usize;
                                            if let Some(popup) = self.next_popup.take() {
                                                if idx < popup.options.len() {
                                                    self.do_resume(popup.session, popup.options[idx]);
                                                }
                                            }
                                        }
                                        KeyCode::Enter => {
                                            if let Some(popup) = self.next_popup.take() {
                                                let target = popup.options[popup.selected];
                                                self.do_resume(popup.session, target);
                                            }
                                        }
                                        _ => {}
                                    }
                                } else if self.cross_tool_popup {
                                    match key.code {
                                        KeyCode::Esc => {
                                            self.cross_tool_popup = false;
                                            self.cross_tool_targets.clear();
                                            self.cross_tool_selected = 0;
                                        }
                                        KeyCode::Up | KeyCode::Char('k') => {
                                            if !self.cross_tool_targets.is_empty() {
                                                if self.cross_tool_selected == 0 {
                                                    self.cross_tool_selected =
                                                        self.cross_tool_targets.len() - 1;
                                                } else {
                                                    self.cross_tool_selected -= 1;
                                                }
                                            }
                                        }
                                        KeyCode::Down | KeyCode::Char('j') => {
                                            if !self.cross_tool_targets.is_empty() {
                                                self.cross_tool_selected = (self.cross_tool_selected
                                                    + 1)
                                                    % self.cross_tool_targets.len();
                                            }
                                        }
                                        KeyCode::Enter => {
                                            if let Some(&target) = self
                                                .cross_tool_targets
                                                .get(self.cross_tool_selected)
                                            {
                                                if let Some(session) = self.selected_session().cloned() {
                                                    self.cross_tool_popup = false;
                                                    self.do_resume(session, target);
                                                }
                                            }
                                        }
                                        _ => {}
                                    }
                                } else {
                                    match self.mode {
                                        AppMode::Search => match key.code {
                                            KeyCode::Esc => {
                                                self.mode = AppMode::Normal;
                                                self.search_query.clear();
                                                self.list_state.select(Some(0));
                                            }
                                            KeyCode::Backspace => {
                                                self.search_query.pop();
                                                self.list_state.select(Some(0));
                                            }
                                            KeyCode::Char(c) => {
                                                self.search_query.push(c);
                                                self.list_state.select(Some(0));
                                            }
                                            _ => {}
                                        },
                                        AppMode::Normal => match key.code {
                                            KeyCode::Char('q') => self.should_quit = true,
                                            KeyCode::Esc => self.should_quit = true,
                                            KeyCode::Up | KeyCode::Char('k') => {
                                                self.move_selection(-1)
                                            }
                                            KeyCode::Down | KeyCode::Char('j') => {
                                                self.move_selection(1)
                                            }
                                            KeyCode::Tab => self.cycle_filter(),
                                            KeyCode::BackTab => self.cycle_filter_backward(),
                                            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                                self.reload_sessions();
                                            }
                                            KeyCode::Char('/') => {
                                                self.mode = AppMode::Search;
                                                self.search_query.clear();
                                            }
                                            KeyCode::Char('f') => {
                                                self.view = View::FlowRunner;
                                                self.flow_runner.start("brainstorm and implement");
                                            }
                                            KeyCode::Char('r') => {
                                                if let Some(session) = self.selected_session() {
                                                    let source = session.source;
                                                    let targets: Vec<SessionSource> = ALL_SOURCES
                                                        .iter()
                                                        .copied()
                                                        .filter(|&s| s != source)
                                                        .collect();
                                                    if !targets.is_empty() {
                                                        self.cross_tool_targets = targets;
                                                        self.cross_tool_selected = 0;
                                                        self.cross_tool_popup = true;
                                                    }
                                                }
                                            }
                                            KeyCode::Char('e') => {
                                                if let Some(session) = self.selected_session() {
                                                    let source = session.source;
                                                    let id = session.id.clone();
                                                    let original_path = session
                                                        .original_path
                                                        .to_string_lossy()
                                                        .into_owned();
                                                    let cwd =
                                                        session.cwd.to_string_lossy().into_owned();
                                                    let mut state = EmbeddedState::new();
                                                    if state
                                                        .start(source, &id, &original_path, &cwd)
                                                        .is_ok()
                                                    {
                                                        self.embedded = Some(state);
                                                        self.view = View::Embedded;
                                                    }
                                                }
                                            }
                                            // Enter opens the "what next?" popup
                                            KeyCode::Enter => {
                                                if let Some(session) = self.selected_session().cloned() {
                                                    let options = next_popup_options(session.source);
                                                    self.next_popup = Some(NextPopupState {
                                                        session,
                                                        options,
                                                        selected: 0,
                                                    });
                                                }
                                            }
                                            _ => {}
                                        },
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Tick flow runner if active
            if matches!(self.view, View::FlowRunner) {
                self.flow_runner.tick();
            }

            // Poll PTY output on every tick so the render loop stays responsive.
            if matches!(self.view, View::Embedded) {
                if let Some(ref mut emb) = self.embedded {
                    emb.poll_output();
                    if !emb.is_running {
                        self.view = View::SessionHub;
                        self.embedded = None;
                    }
                }
            }

            if self.should_quit {
                return Ok(());
            }
        }
    }

    /// Perform the actual resume: write SCP, tear down TUI, and exec the target tool.
    fn do_resume(&mut self, session: UnifiedSession, target: SessionSource) {
        // Auto-write SCP context for the target tool to pick up
        let scp_path = orc_core::session::scp::default_scp_path(&session.cwd);
        let ctx = make_minimal_context(&session);
        let _ = orc_core::session::scp::write_scp(&scp_path, &ctx);

        let _ = disable_raw_mode();
        let _ = execute!(stdout(), LeaveAlternateScreen);

        if target == session.source {
            // Same tool — direct resume
            let cmd = build_resume_command(
                session.source,
                &session.id,
                &session.original_path.to_string_lossy(),
                &session.cwd.to_string_lossy(),
            );
            let _ = spawn_and_exit(cmd);
        } else {
            // Cross-tool resume — use cross_tool_resume
            let _ = cross_tool_resume(&ctx, target);
        }
        std::process::exit(1);
    }

    fn move_selection(&mut self, delta: i32) {
        let count = self.filtered_count();
        if count == 0 {
            return;
        }
        let current = self.list_state.selected().unwrap_or(0) as i32;
        let next = (current + delta).rem_euclid(count as i32) as usize;
        self.list_state.select(Some(next));
    }

    fn filtered_count(&self) -> usize {
        self.sessions
            .iter()
            .filter(|s| {
                if let Some(f) = self.filter {
                    if s.source != f {
                        return false;
                    }
                }
                if !self.search_query.is_empty() {
                    let q = self.search_query.to_lowercase();
                    let ok = s.summary.to_lowercase().contains(&q)
                        || s.cwd.to_string_lossy().to_lowercase().contains(&q)
                        || s.id.to_lowercase().contains(&q);
                    if !ok {
                        return false;
                    }
                }
                true
            })
            .count()
    }

    fn selected_session(&self) -> Option<&UnifiedSession> {
        let idx = self.list_state.selected()?;
        let filtered: Vec<&UnifiedSession> = self
            .sessions
            .iter()
            .filter(|s| {
                if let Some(f) = self.filter {
                    if s.source != f {
                        return false;
                    }
                }
                if !self.search_query.is_empty() {
                    let q = self.search_query.to_lowercase();
                    let ok = s.summary.to_lowercase().contains(&q)
                        || s.cwd.to_string_lossy().to_lowercase().contains(&q)
                        || s.id.to_lowercase().contains(&q);
                    if !ok {
                        return false;
                    }
                }
                true
            })
            .collect();
        filtered.get(idx).copied()
    }

    fn cycle_filter(&mut self) {
        self.filter = match self.filter {
            None => Some(SessionSource::Claude),
            Some(SessionSource::Claude) => Some(SessionSource::Codex),
            Some(SessionSource::Codex) => Some(SessionSource::Gemini),
            Some(SessionSource::Gemini) => Some(SessionSource::Copilot),
            Some(SessionSource::Copilot) => Some(SessionSource::OpenCode),
            Some(SessionSource::OpenCode) => Some(SessionSource::Kimi),
            Some(SessionSource::Kimi) => Some(SessionSource::Amp),
            Some(SessionSource::Amp) => Some(SessionSource::Droid),
            Some(SessionSource::Droid) | Some(SessionSource::Scp) => None,
        };
        self.list_state.select(Some(0));
    }

    fn cycle_filter_backward(&mut self) {
        self.filter = match self.filter {
            None => Some(SessionSource::Droid),
            Some(SessionSource::Droid) => Some(SessionSource::Amp),
            Some(SessionSource::Amp) => Some(SessionSource::Kimi),
            Some(SessionSource::Kimi) => Some(SessionSource::OpenCode),
            Some(SessionSource::OpenCode) => Some(SessionSource::Copilot),
            Some(SessionSource::Copilot) => Some(SessionSource::Gemini),
            Some(SessionSource::Gemini) => Some(SessionSource::Codex),
            Some(SessionSource::Codex) => Some(SessionSource::Claude),
            Some(SessionSource::Claude) | Some(SessionSource::Scp) => None,
        };
        self.list_state.select(Some(0));
    }

    fn reload_sessions(&mut self) {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to build tokio runtime");
        let sessions = rt.block_on(async {
            let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
            let claude_root = home.join(".claude").join("projects");
            let codex_root = home.join(".codex").join("sessions");
            let index = build_index_full(&claude_root, &codex_root, None, None, None)
                .await
                .unwrap_or_else(|_| SessionIndex {
                    sessions: vec![],
                    built_at: chrono::Utc::now(),
                });
            if let Ok(index_path) = default_index_path() {
                let _ = save_index(&index_path, &index);
            }
            index.sessions
        });
        let count = sessions.len();
        self.sessions = sessions;
        self.list_state.select(Some(0));
        self.status_msg = format!("↺ {} sessions", count);
    }

    fn render(&mut self, frame: &mut ratatui::Frame) {
        let area = frame.area();

        match self.view {
            View::Embedded => {
                if let Some(ref emb) = self.embedded {
                    let widget = PtyViewportWidget { lines: &emb.lines };
                    frame.render_widget(widget, area);
                }
            }
            View::FlowRunner => {
                render_flow_runner(&self.flow_runner, frame, area);
            }
            View::SessionHub => {
                let chunks = Layout::vertical([Constraint::Min(1), Constraint::Length(1)])
                    .split(area);

                let title = self.list_title();
                let widget = SessionListWidget {
                    sessions: &self.sessions,
                    filter: self.filter,
                    search: &self.search_query,
                    title: &title,
                    project_cwd: self.project_cwd.as_deref(),
                };
                frame.render_stateful_widget(widget, chunks[0], &mut self.list_state);

                let footer = match self.mode {
                    AppMode::Normal => FooterWidget::new_normal(self.filter, &self.status_msg),
                    AppMode::Search => FooterWidget::new_search(&self.search_query),
                };
                frame.render_widget(footer, chunks[1]);

                if self.cross_tool_popup {
                    self.render_cross_tool_popup(frame, area);
                }

                if self.next_popup.is_some() {
                    self.render_next_popup(frame, area);
                }
            }
        }
    }

    fn render_cross_tool_popup(&self, frame: &mut ratatui::Frame, area: Rect) {
        let popup_width: u16 = 44;
        let popup_height: u16 = (self.cross_tool_targets.len() as u16).saturating_add(5);
        let popup_area = centered_rect(popup_width, popup_height, area);

        frame.render_widget(Clear, popup_area);

        let block = Block::default()
            .title(" Resume in: ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan));

        let inner = block.inner(popup_area);
        frame.render_widget(block, popup_area);

        let items: Vec<ListItem> = self
            .cross_tool_targets
            .iter()
            .enumerate()
            .map(|(i, &src)| {
                let name = source_display_name(src);
                if i == self.cross_tool_selected {
                    ListItem::new(Line::from(vec![
                        Span::styled(" \u{25b6} ", Style::default().fg(Color::Yellow)),
                        Span::styled(
                            name,
                            Style::default()
                                .fg(Color::White)
                                .add_modifier(Modifier::BOLD),
                        ),
                    ]))
                } else {
                    ListItem::new(Line::from(vec![
                        Span::raw("   "),
                        Span::styled(name, Style::default().fg(Color::DarkGray)),
                    ]))
                }
            })
            .collect();

        let inner_chunks =
            Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(inner);

        let mut list_state = ListState::default();
        list_state.select(Some(self.cross_tool_selected));

        let list = List::new(items);
        frame.render_stateful_widget(list, inner_chunks[0], &mut list_state);

        let hint = Paragraph::new(Line::from(vec![
            Span::styled("enter", Style::default().fg(Color::White)),
            Span::styled(" select  ", Style::default().fg(Color::DarkGray)),
            Span::styled("esc", Style::default().fg(Color::White)),
            Span::styled(" cancel", Style::default().fg(Color::DarkGray)),
        ]));
        frame.render_widget(hint, inner_chunks[1]);
    }

    fn render_next_popup(&self, frame: &mut ratatui::Frame, area: Rect) {
        let popup = match &self.next_popup {
            Some(p) => p,
            None => return,
        };

        let popup_area = centered_rect(54, (popup.options.len() as u16) + 6, area);
        frame.render_widget(Clear, popup_area);

        let mut lines = vec![Line::from("")];

        // Session info line
        let summary: String = popup.session.summary.chars().take(35).collect();
        let dir = popup
            .session
            .cwd
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?");
        let session_info = format!("  {}  ·  {}", summary, dir);
        lines.push(Line::from(session_info));
        lines.push(Line::from(""));

        for (i, &src) in popup.options.iter().enumerate() {
            let label = source_display_name(src);
            let hint = if src == popup.session.source { "  (same tool)" } else { "" };
            let text = format!("  {}  {}{}", i + 1, label, hint);
            if i == popup.selected {
                lines.push(Line::from(Span::styled(
                    text,
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )));
            } else {
                lines.push(Line::from(text));
            }
        }

        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Esc  cancel",
            Style::default().fg(Color::DarkGray),
        )));

        frame.render_widget(
            Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Resume session "),
            ),
            popup_area,
        );
    }

    fn list_title(&self) -> String {
        let total = self.sessions.len();
        if let Some(cwd) = &self.project_cwd {
            let local = self.sessions.iter().filter(|s| s.cwd == *cwd).count();
            let project = cwd.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("?");
            if local > 0 {
                return format!("{} — {} local, {} total", project, local, total);
            }
        }
        format!("Sessions ({})", total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_local_session_count() {
        use orc_core::session::types::{SessionSource, UnifiedSession};
        use std::path::PathBuf;
        use chrono::Utc;

        let cwd = PathBuf::from("/my/project");
        let sessions = vec![
            UnifiedSession {
                id: "aaa".to_string(),
                source: SessionSource::Claude,
                cwd: PathBuf::from("/my/project"),
                repo: None,
                summary: "".to_string(),
                files_modified: vec![],
                updated_at: Utc::now(),
                created_at: Utc::now(),
                message_count: 0,
                original_path: PathBuf::from("/tmp/a.jsonl"),
            },
            UnifiedSession {
                id: "bbb".to_string(),
                source: SessionSource::Claude,
                cwd: PathBuf::from("/other"),
                repo: None,
                summary: "".to_string(),
                files_modified: vec![],
                updated_at: Utc::now(),
                created_at: Utc::now(),
                message_count: 0,
                original_path: PathBuf::from("/tmp/b.jsonl"),
            },
        ];
        let count = sessions.iter().filter(|s| s.cwd == cwd).count();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_scp_path_from_session() {
        use std::path::PathBuf;
        use orc_core::session::scp::default_scp_path;
        let cwd = PathBuf::from("/my/project");
        let path = default_scp_path(&cwd);
        assert_eq!(path, PathBuf::from("/my/project/.orc/context.json"));
    }

    #[test]
    fn test_next_popup_options_same_tool_first() {
        use orc_core::session::types::SessionSource;
        let source = SessionSource::Claude;
        let opts = next_popup_options(source);
        assert_eq!(opts[0], SessionSource::Claude);
        assert!(opts.len() >= 2);
        // All options are distinct
        let unique: std::collections::HashSet<_> = opts.iter().collect();
        assert_eq!(unique.len(), opts.len());
    }

    #[test]
    fn test_next_popup_options_excludes_unknown() {
        use orc_core::session::types::SessionSource;
        let opts = next_popup_options(SessionSource::Codex);
        // codex should be first
        assert_eq!(opts[0], SessionSource::Codex);
        // No Scp source in the options
        assert!(!opts.contains(&SessionSource::Scp));
    }
}
