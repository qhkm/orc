use orc_core::session::types::SessionSource;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;
use crate::theme::footer_style;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    Normal,
    Search,
}

pub struct FooterWidget {
    pub mode: AppMode,
    pub search_query: String,
    pub filter: Option<SessionSource>,
    pub status: String,
}

impl FooterWidget {
    pub fn new_normal(filter: Option<SessionSource>, status: impl Into<String>) -> Self {
        Self {
            mode: AppMode::Normal,
            search_query: String::new(),
            filter,
            status: status.into(),
        }
    }

    pub fn new_search(query: impl Into<String>) -> Self {
        Self {
            mode: AppMode::Search,
            search_query: query.into(),
            filter: None,
            status: String::new(),
        }
    }

    fn filter_label(filter: Option<SessionSource>) -> String {
        match filter {
            None => "all".to_string(),
            Some(SessionSource::Claude) => "claude".to_string(),
            Some(SessionSource::Codex) => "codex".to_string(),
            Some(SessionSource::Gemini) => "gemini".to_string(),
            Some(SessionSource::Copilot) => "copilot".to_string(),
            Some(SessionSource::OpenCode) => "opencode".to_string(),
            Some(SessionSource::Kimi) => "kimi".to_string(),
            Some(SessionSource::Amp) => "amp".to_string(),
            Some(SessionSource::Droid) => "droid".to_string(),
            Some(SessionSource::Scp) => "scp".to_string(),
        }
    }

    fn normal_hints(filter: Option<SessionSource>, width: u16) -> Vec<(&'static str, String)> {
        let filter_label = Self::filter_label(filter);
        let mut hints: Vec<(&'static str, String)> = vec![
            ("↑↓", "navigate".to_string()),
            ("enter", "resume".to_string()),
            ("r", "resume-in".to_string()),
            ("tab", format!("filter: {}", filter_label)),
            ("/", "search".to_string()),
            ("ctrl+r", "refresh".to_string()),
            ("e", "embed".to_string()),
            ("x", "export".to_string()),
            ("f", "flow".to_string()),
            ("q", "quit".to_string()),
        ];
        // Collapse hints based on width
        if width < 120 {
            hints.retain(|(k, _)| ["↑↓", "enter", "r", "tab", "/", "ctrl+r", "q"].contains(k));
        }
        if width < 80 {
            hints.retain(|(k, _)| ["↑↓", "enter", "q"].contains(k));
        }
        hints
    }
}

impl Widget for FooterWidget {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if self.mode == AppMode::Search {
            let line = Line::from(vec![
                Span::styled("/ ", Style::default().fg(Color::Yellow)),
                Span::raw(self.search_query.clone()),
                Span::styled("▌", Style::default().fg(Color::Yellow)),
                Span::styled("   esc cancel", footer_style()),
            ]);
            line.render(area, buf);
            return;
        }

        let hints = Self::normal_hints(self.filter, area.width);
        let mut spans: Vec<Span> = Vec::new();
        for (i, (key, desc)) in hints.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled("  ", footer_style()));
            }
            spans.push(Span::styled(key.to_string(), Style::default().fg(Color::White)));
            spans.push(Span::styled(format!(" {}", desc), footer_style()));
        }
        if !self.status.is_empty() {
            spans.push(Span::styled(format!("   {}", self.status), Style::default().fg(Color::Green)));
        }
        Line::from(spans).render(area, buf);
    }
}
