use orc_core::flow::types::Stage;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

pub struct StageHeaderWidget<'a> {
    pub stages: &'a [Stage],
    pub current_stage_idx: usize,
    pub elapsed_secs: u64,
}

impl<'a> Widget for StageHeaderWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if self.stages.is_empty() {
            return;
        }

        let mut spans: Vec<Span> = Vec::new();

        for (i, stage) in self.stages.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled(" → ", Style::default().fg(Color::DarkGray)));
            }

            let label = format!("[{}:{}]", stage.provider, stage.intent);

            if i < self.current_stage_idx {
                // Completed
                spans.push(Span::styled(label, Style::default().fg(Color::Green)));
            } else if i == self.current_stage_idx {
                // Active
                spans.push(Span::styled(
                    label,
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ));
            } else {
                // Pending
                spans.push(Span::styled(label, Style::default().fg(Color::DarkGray)));
            }
        }

        // Add elapsed time
        let elapsed = format!("  {}s", self.elapsed_secs);
        spans.push(Span::styled(elapsed, Style::default().fg(Color::DarkGray)));

        Line::from(spans).render(area, buf);
    }
}
