use ratatui::style::{Color, Modifier, Style};

pub fn source_color(source: &str) -> Color {
    match source {
        "claude" => Color::Rgb(204, 120, 50),
        "codex" => Color::Rgb(0, 164, 84),
        "gemini" => Color::Rgb(66, 133, 244),
        "copilot" => Color::Rgb(140, 86, 245),
        "open_code" => Color::Rgb(220, 220, 100),
        "kimi" => Color::Rgb(0, 191, 255),
        "amp" => Color::Rgb(255, 100, 100),
        "droid" => Color::Rgb(180, 100, 200),
        _ => Color::Gray,
    }
}

pub fn source_label(source: &str) -> &'static str {
    match source {
        "claude" => "claude",
        "codex" => "codex",
        "gemini" => "gemini",
        "copilot" => "copilot",
        "open_code" => "opencode",
        "kimi" => "kimi",
        "amp" => "amp",
        "droid" => "droid",
        "scp" => "scp",
        _ => "?",
    }
}

pub fn footer_style() -> Style {
    Style::default().fg(Color::DarkGray)
}

pub fn selected_style() -> Style {
    Style::default().add_modifier(Modifier::REVERSED)
}

pub fn badge_style(source: &str) -> Style {
    Style::default().fg(source_color(source)).add_modifier(Modifier::BOLD)
}
