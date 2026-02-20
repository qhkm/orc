use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

static SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub struct SpinnerWidget {
    pub tick: u64,
    pub label: String,
    pub done: bool,
}

impl SpinnerWidget {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            tick: 0,
            label: label.into(),
            done: false,
        }
    }

    pub fn with_tick(mut self, tick: u64) -> Self {
        self.tick = tick;
        self
    }

    pub fn done(mut self) -> Self {
        self.done = true;
        self
    }
}

impl Widget for SpinnerWidget {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 || area.width == 0 {
            return;
        }
        let frame = if self.done {
            "✓"
        } else {
            SPINNER_FRAMES[(self.tick as usize) % SPINNER_FRAMES.len()]
        };

        let style = if self.done {
            Style::default().fg(Color::Green)
        } else {
            Style::default().fg(Color::Cyan)
        };

        let line = Line::from(vec![
            Span::styled(format!("{} ", frame), style),
            Span::raw(self.label.clone()),
        ]);
        line.render(area, buf);
    }
}
