use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::Widget;

/// A widget that renders a `vt100::Screen` to a ratatui `Buffer`.
///
/// Maps vt100 terminal cells (colors, bold, italic, underline) to ratatui
/// styled cells, preserving the visual output of the PTY process.
pub struct PtyScreenWidget<'a> {
    pub screen: &'a vt100::Screen,
    pub scroll_offset: u16,
}

impl<'a> Widget for PtyScreenWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let screen = self.screen;
        let rows = screen.size().0;
        let cols = screen.size().1;

        // For now, render the visible screen cells directly.
        // scroll_offset is reserved for future scrollback support.
        for y in 0..area.height.min(rows) {
            for x in 0..area.width.min(cols) {
                let buf_x = area.x + x;
                let buf_y = area.y + y;

                if let Some(cell) = screen.cell(y, x) {
                    let ch = cell.contents();
                    let fg = map_vt100_color(cell.fgcolor());
                    let bg = map_vt100_color(cell.bgcolor());

                    let mut modifier = Modifier::empty();
                    if cell.bold() {
                        modifier |= Modifier::BOLD;
                    }
                    if cell.italic() {
                        modifier |= Modifier::ITALIC;
                    }
                    if cell.underline() {
                        modifier |= Modifier::UNDERLINED;
                    }
                    if cell.inverse() {
                        modifier |= Modifier::REVERSED;
                    }

                    let style = Style::default().fg(fg).bg(bg).add_modifier(modifier);

                    let c = if ch.is_empty() { " " } else { &ch };
                    if let Some(first_char) = c.chars().next() {
                        buf.set_string(buf_x, buf_y, &first_char.to_string(), style);
                    }
                }
            }
        }
    }
}

/// Map a vt100 `Color` to a ratatui `Color`.
fn map_vt100_color(color: vt100::Color) -> Color {
    match color {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(idx) => Color::Indexed(idx),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_map_vt100_color_default() {
        assert_eq!(map_vt100_color(vt100::Color::Default), Color::Reset);
    }

    #[test]
    fn test_map_vt100_color_indexed() {
        assert_eq!(map_vt100_color(vt100::Color::Idx(1)), Color::Indexed(1));
        assert_eq!(map_vt100_color(vt100::Color::Idx(255)), Color::Indexed(255));
    }

    #[test]
    fn test_map_vt100_color_rgb() {
        assert_eq!(
            map_vt100_color(vt100::Color::Rgb(255, 128, 0)),
            Color::Rgb(255, 128, 0)
        );
    }

    #[test]
    fn test_pty_screen_widget_renders_without_panic() {
        let mut parser = vt100::Parser::new(24, 80, 0);
        parser.process(b"Hello, world!\r\n\x1b[1mBold text\x1b[0m");

        let widget = PtyScreenWidget {
            screen: parser.screen(),
            scroll_offset: 0,
        };

        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf);

        // Verify first cell has 'H'.
        let cell = buf.cell((0, 0)).unwrap();
        assert_eq!(cell.symbol(), "H");
    }

    #[test]
    fn test_pty_screen_widget_colors() {
        let mut parser = vt100::Parser::new(24, 80, 0);
        // Red foreground text: ESC[31m
        parser.process(b"\x1b[31mRed\x1b[0m");

        let widget = PtyScreenWidget {
            screen: parser.screen(),
            scroll_offset: 0,
        };

        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf);

        let cell = buf.cell((0, 0)).unwrap();
        assert_eq!(cell.symbol(), "R");
        // Red is typically index 1 in standard terminal colors.
        assert_eq!(cell.fg, Color::Indexed(1));
    }
}
