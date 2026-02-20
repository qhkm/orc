use orc_core::session::types::{SessionSource, UnifiedSession};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, StatefulWidget};
use std::path::Path;
use crate::theme::{badge_style, selected_style, source_label};

pub struct SessionListWidget<'a> {
    pub sessions: &'a [UnifiedSession],
    pub filter: Option<SessionSource>,
    pub search: &'a str,
    pub title: &'a str,
    pub project_cwd: Option<&'a Path>,
}

pub struct SessionListState {
    pub list_state: ListState,
    pub scroll_offset: usize,
}

impl SessionListState {
    pub fn new() -> Self {
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        Self { list_state, scroll_offset: 0 }
    }

    pub fn selected(&self) -> Option<usize> {
        self.list_state.selected()
    }

    pub fn select(&mut self, idx: Option<usize>) {
        self.list_state.select(idx);
    }
}

impl Default for SessionListState {
    fn default() -> Self {
        Self::new()
    }
}

fn format_time_ago(session: &UnifiedSession) -> String {
    let now = chrono::Utc::now();
    let elapsed = now.signed_duration_since(session.updated_at);
    let secs = elapsed.num_seconds();
    if secs < 60 {
        "just now".to_string()
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86400)
    }
}

fn project_name(session: &UnifiedSession) -> String {
    session.cwd
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("?")
        .to_string()
}

fn short_id(id: &str) -> &str {
    &id[..id.len().min(8)]
}

fn source_key(source: &SessionSource) -> &'static str {
    match source {
        SessionSource::Claude => "claude",
        SessionSource::Codex => "codex",
        SessionSource::Gemini => "gemini",
        SessionSource::Copilot => "copilot",
        SessionSource::OpenCode => "open_code",
        SessionSource::Kimi => "kimi",
        SessionSource::Amp => "amp",
        SessionSource::Droid => "droid",
        SessionSource::Scp => "scp",
    }
}

/// Returns a compact files string: "auth.ts  middleware.ts" (basenames, max 2 files).
fn files_badge(session: &UnifiedSession) -> String {
    let top: Vec<String> = session.files_modified
        .iter()
        .take(2)
        .map(|p| {
            std::path::Path::new(p)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(p.as_str())
                .to_string()
        })
        .collect();
    top.join("  ")
}

impl<'a> SessionListWidget<'a> {
    pub fn filtered_sessions(&self) -> Vec<&UnifiedSession> {
        let mut matched: Vec<&UnifiedSession> = Vec::new();
        let mut rest: Vec<&UnifiedSession> = Vec::new();

        for s in self.sessions.iter() {
            // Source filter
            if let Some(f) = self.filter {
                if s.source != f {
                    continue;
                }
            }
            // Search filter
            if !self.search.is_empty() {
                let q = self.search.to_lowercase();
                let hits = s.summary.to_lowercase().contains(&q)
                    || s.cwd.to_string_lossy().to_lowercase().contains(&q)
                    || s.id.to_lowercase().contains(&q);
                if !hits {
                    continue;
                }
            }
            // CWD sort bucket
            if let Some(cwd) = self.project_cwd {
                if s.cwd == cwd {
                    matched.push(s);
                    continue;
                }
            }
            rest.push(s);
        }

        matched.extend(rest);
        matched
    }
}

impl<'a> StatefulWidget for SessionListWidget<'a> {
    type State = SessionListState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let filtered = self.filtered_sessions();

        let items: Vec<ListItem> = filtered
            .iter()
            .map(|session| {
                let src_key = source_key(&session.source);
                let badge = format!("[{}]", source_label(src_key));
                let time = format_time_ago(session);
                let project = project_name(session);
                let summary = if session.summary.len() > 40 {
                    format!("{}...", &session.summary[..37])
                } else {
                    session.summary.clone()
                };
                let files = files_badge(session);
                let id = short_id(&session.id);

                let line = Line::from(vec![
                    Span::styled(format!("{:<12}", badge), badge_style(src_key)),
                    Span::raw("  "),
                    Span::styled(format!("{:<10}", time), Style::default().fg(Color::DarkGray)),
                    Span::raw("  "),
                    Span::styled(format!("{:<20}", project), Style::default().fg(Color::Cyan)),
                    Span::raw("  "),
                    Span::styled(format!("{:<42}", summary), Style::default()),
                    Span::raw("  "),
                    Span::styled(format!("{:<28}", files), Style::default().fg(Color::DarkGray)),  // files
                    Span::raw("  "),
                    Span::styled(id.to_string(), Style::default().fg(Color::DarkGray)),
                ]);

                ListItem::new(line)
            })
            .collect();

        let total = items.len();
        let title = format!(" {} ({} sessions) ", self.title, total);

        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title(title))
            .highlight_style(selected_style())
            .highlight_symbol("▶ ");

        StatefulWidget::render(list, area, buf, &mut state.list_state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use chrono::Utc;
    use orc_core::session::types::{SessionSource, UnifiedSession};

    fn make_session(id: &str, cwd: &str, source: SessionSource) -> UnifiedSession {
        UnifiedSession {
            id: id.to_string(),
            source,
            cwd: PathBuf::from(cwd),
            repo: None,
            summary: "test".to_string(),
            updated_at: Utc::now(),
            created_at: Utc::now(),
            message_count: 1,
            files_modified: vec![],
            original_path: PathBuf::from("/tmp/fake.jsonl"),
        }
    }

    #[test]
    fn test_project_cwd_sessions_sorted_first() {
        let sessions = vec![
            make_session("aaa", "/other/project", SessionSource::Claude),
            make_session("bbb", "/my/project", SessionSource::Claude),
            make_session("ccc", "/my/project", SessionSource::Codex),
        ];
        let widget = SessionListWidget {
            sessions: &sessions,
            filter: None,
            search: "",
            title: "test",
            project_cwd: Some(&PathBuf::from("/my/project")),
        };
        let filtered = widget.filtered_sessions();
        // bbb and ccc should come first (they match /my/project)
        assert_eq!(filtered[0].id, "bbb");
        assert_eq!(filtered[1].id, "ccc");
        assert_eq!(filtered[2].id, "aaa");
    }

    #[test]
    fn test_no_project_cwd_order_unchanged() {
        let sessions = vec![
            make_session("aaa", "/other", SessionSource::Claude),
            make_session("bbb", "/my/project", SessionSource::Claude),
        ];
        let widget = SessionListWidget {
            sessions: &sessions,
            filter: None,
            search: "",
            title: "test",
            project_cwd: None,
        };
        let filtered = widget.filtered_sessions();
        assert_eq!(filtered[0].id, "aaa");
        assert_eq!(filtered[1].id, "bbb");
    }

    #[test]
    fn test_row_shows_files_when_present() {
        let session = UnifiedSession {
            id: "abc12345".to_string(),
            source: SessionSource::Claude,
            cwd: PathBuf::from("/my/project"),
            repo: None,
            summary: "add auth".to_string(),
            files_modified: vec!["src/auth.ts".to_string(), "src/middleware.ts".to_string()],
            updated_at: Utc::now(),
            created_at: Utc::now(),
            message_count: 5,
            original_path: PathBuf::from("/tmp/abc.jsonl"),
        };

        let files_str = files_badge(&session);
        assert!(files_str.contains("auth.ts"), "got: {}", files_str);
    }

    #[test]
    fn test_row_shows_empty_when_no_files() {
        let session = UnifiedSession {
            id: "abc12345".to_string(),
            source: SessionSource::Claude,
            cwd: PathBuf::from("/my/project"),
            repo: None,
            summary: "add auth".to_string(),
            files_modified: vec![],
            updated_at: Utc::now(),
            created_at: Utc::now(),
            message_count: 5,
            original_path: PathBuf::from("/tmp/abc.jsonl"),
        };

        let files_str = files_badge(&session);
        assert!(files_str.is_empty(), "got: {}", files_str);
    }
}
