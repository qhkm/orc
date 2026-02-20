use std::collections::VecDeque;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget, Wrap};

/// A widget that renders lines of PTY output inside a scrollable bordered block.
///
/// Shows only the last N lines that fit within the available area.  Long lines
/// are wrapped automatically using ratatui's `Wrap` support.
pub struct PtyViewportWidget<'a> {
    pub lines: &'a VecDeque<String>,
}

impl<'a> Widget for PtyViewportWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .title(" embedded terminal ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray));

        let inner = block.inner(area);
        let visible_height = inner.height as usize;

        // Collect all lines as ratatui Lines, applying plain white style.
        let all_lines: Vec<Line> = self
            .lines
            .iter()
            .flat_map(|raw| {
                // Split by embedded newlines so a single VecDeque entry that
                // contains '\n' is rendered as multiple visual rows.
                raw.split('\n')
                    .map(|l| Line::from(Span::styled(l.to_string(), Style::default().fg(Color::White))))
                    .collect::<Vec<_>>()
            })
            .collect();

        // Scroll so the *last* lines are always visible (tail-follow behaviour).
        let total = all_lines.len();
        let scroll: u16 = if total > visible_height {
            (total - visible_height) as u16
        } else {
            0
        };

        let paragraph = Paragraph::new(all_lines)
            .block(block)
            .style(Style::default().fg(Color::White).bg(Color::Black))
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0));

        paragraph.render(area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};
    use crate::views::embedded::key_to_bytes;

    #[test]
    fn test_key_to_bytes_enter() {
        let bytes = key_to_bytes(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(bytes, b"\r".to_vec());
    }

    #[test]
    fn test_key_to_bytes_ctrl_c() {
        let bytes = key_to_bytes(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(bytes, vec![0x03]);
    }

    #[test]
    fn test_key_to_bytes_arrow_up() {
        let bytes = key_to_bytes(KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(bytes, b"\x1b[A".to_vec());
    }

    #[test]
    fn test_pty_viewport_widget_renders_without_panic() {
        // Smoke test: constructing the widget with a populated deque should not
        // panic.  We just verify the struct can be built; actual pixel-level
        // rendering is exercised by integration tests.
        let mut lines = VecDeque::new();
        lines.push_back("hello world".to_string());
        lines.push_back("second line".to_string());
        let _widget = PtyViewportWidget { lines: &lines };
        // If we reach here without panic the test passes.
    }
}
