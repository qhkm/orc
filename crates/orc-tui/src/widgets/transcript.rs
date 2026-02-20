use orc_core::session::types::StreamEvent;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget, Wrap};

#[derive(Debug, Clone)]
pub enum TranscriptEntry {
    Committed(String, String), // (role/label, content)
    Live(String),              // currently streaming text
    ToolUse { name: String, input: String },
    ToolResult { name: String, output: String },
    Reasoning(String),
    Status(String),
}

pub struct TranscriptWidget<'a> {
    pub entries: &'a [TranscriptEntry],
    pub scroll_offset: usize,
    pub is_live: bool,
    pub title: &'a str,
}

pub struct TranscriptState {
    pub entries: Vec<TranscriptEntry>,
    pub scroll_offset: usize,
    pub live_text: String,
}

impl TranscriptState {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            scroll_offset: 0,
            live_text: String::new(),
        }
    }

    pub fn push_event(&mut self, event: StreamEvent) {
        match event {
            StreamEvent::Text { content } => {
                self.live_text.push_str(&content);
            }
            StreamEvent::ToolStart { name, input } => {
                // Commit any live text first
                if !self.live_text.is_empty() {
                    let text = std::mem::take(&mut self.live_text);
                    self.entries.push(TranscriptEntry::Committed("assistant".into(), text));
                }
                self.entries.push(TranscriptEntry::ToolUse { name, input });
            }
            StreamEvent::ToolEnd { name, output } => {
                self.entries.push(TranscriptEntry::ToolResult { name, output });
            }
            StreamEvent::Reasoning { content } => {
                self.entries.push(TranscriptEntry::Reasoning(content));
            }
            StreamEvent::Status { message } => {
                self.entries.push(TranscriptEntry::Status(message));
            }
            StreamEvent::Done => {
                // Commit final live text
                if !self.live_text.is_empty() {
                    let text = std::mem::take(&mut self.live_text);
                    self.entries.push(TranscriptEntry::Committed("assistant".into(), text));
                }
            }
            StreamEvent::Error { message } => {
                self.entries.push(TranscriptEntry::Status(format!("Error: {}", message)));
            }
        }
    }

    pub fn scroll_down(&mut self, amount: usize) {
        self.scroll_offset = self.scroll_offset.saturating_add(amount);
    }

    pub fn scroll_up(&mut self, amount: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(amount);
    }
}

impl Default for TranscriptState {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Widget for TranscriptWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let mut lines: Vec<Line> = Vec::new();

        for entry in self.entries {
            match entry {
                TranscriptEntry::Committed(label, content) => {
                    let style = if label == "user" {
                        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
                    };
                    lines.push(Line::from(Span::styled(format!("{}:", label), style)));
                    for line in content.lines() {
                        lines.push(Line::from(Span::raw(format!("  {}", line))));
                    }
                    lines.push(Line::default());
                }
                TranscriptEntry::Live(text) => {
                    lines.push(Line::from(vec![
                        Span::styled("\u{258c} ", Style::default().fg(Color::Cyan)),
                        Span::raw(text.clone()),
                    ]));
                }
                TranscriptEntry::ToolUse { name, input } => {
                    lines.push(Line::from(vec![
                        Span::styled(format!("  \u{2699} {} ", name), Style::default().fg(Color::Yellow)),
                        Span::styled(
                            input.chars().take(80).collect::<String>(),
                            Style::default().fg(Color::DarkGray),
                        ),
                    ]));
                }
                TranscriptEntry::ToolResult { name, output } => {
                    lines.push(Line::from(vec![
                        Span::styled(format!("  \u{2713} {} ", name), Style::default().fg(Color::Green)),
                        Span::styled(
                            output.chars().take(80).collect::<String>(),
                            Style::default().fg(Color::DarkGray),
                        ),
                    ]));
                }
                TranscriptEntry::Reasoning(text) => {
                    lines.push(Line::from(vec![
                        Span::styled("  \u{1f4ad} ", Style::default().fg(Color::Magenta)),
                        Span::styled(
                            text.chars().take(100).collect::<String>(),
                            Style::default().fg(Color::DarkGray),
                        ),
                    ]));
                }
                TranscriptEntry::Status(msg) => {
                    lines.push(Line::from(Span::styled(
                        format!("  \u{25ba} {}", msg),
                        Style::default().fg(Color::DarkGray),
                    )));
                }
            }
        }

        // Add live indicator when streaming
        if self.is_live && !self.entries.is_empty() {
            lines.push(Line::from(Span::styled(
                "\u{258c} ...",
                Style::default().fg(Color::Cyan),
            )));
        }

        let total_lines = lines.len();
        let visible_height = area.height.saturating_sub(2) as usize;
        let max_scroll = total_lines.saturating_sub(visible_height);
        let scroll = self.scroll_offset.min(max_scroll);

        let title = format!(" {} ", self.title);
        let block = Block::default().borders(Borders::ALL).title(title);

        let paragraph = Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false })
            .scroll((scroll as u16, 0));

        paragraph.render(area, buf);
    }
}
