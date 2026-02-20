use orc_core::flow::types::{CompileOptions, FlowDefinition};
use orc_core::session::types::StreamEvent;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Style};
use ratatui::widgets::Paragraph;
use crate::widgets::stage_header::StageHeaderWidget;
use crate::widgets::transcript::{TranscriptState, TranscriptWidget};

pub struct FlowRunnerState {
    pub flow: Option<FlowDefinition>,
    pub current_stage_idx: usize,
    pub transcript: TranscriptState,
    pub elapsed_ticks: u64,
    pub is_running: bool,
    pub status: String,
}

impl FlowRunnerState {
    pub fn new() -> Self {
        Self {
            flow: None,
            current_stage_idx: 0,
            transcript: TranscriptState::new(),
            elapsed_ticks: 0,
            is_running: false,
            status: String::new(),
        }
    }

    pub fn start(&mut self, prompt: &str) {
        let opts = CompileOptions {
            name: "flow".into(),
            prompt: prompt.into(),
            master_provider: "claude".into(),
            available_providers: vec![],
        };
        if let Ok(def) = orc_core::flow::compile(opts) {
            self.flow = Some(def);
        }
        self.is_running = true;
        self.current_stage_idx = 0;
        self.status = "Running...".into();
    }

    pub fn push_event(&mut self, event: StreamEvent) {
        self.transcript.push_event(event);
    }

    pub fn advance_stage(&mut self) {
        if let Some(flow) = &self.flow {
            if self.current_stage_idx + 1 < flow.stages.len() {
                self.current_stage_idx += 1;
            } else {
                self.is_running = false;
                self.status = "Complete".into();
            }
        }
    }

    pub fn tick(&mut self) {
        if self.is_running {
            self.elapsed_ticks += 1;
        }
    }
}

impl Default for FlowRunnerState {
    fn default() -> Self {
        Self::new()
    }
}

pub fn render_flow_runner(
    state: &FlowRunnerState,
    frame: &mut ratatui::Frame,
    area: ratatui::layout::Rect,
) {
    let chunks = Layout::vertical([
        Constraint::Length(1), // stage header
        Constraint::Min(1),    // transcript
        Constraint::Length(1), // status footer
    ])
    .split(area);

    // Stage header
    if let Some(flow) = &state.flow {
        let header = StageHeaderWidget {
            stages: &flow.stages,
            current_stage_idx: state.current_stage_idx,
            elapsed_secs: state.elapsed_ticks / 20, // 50ms ticks → seconds
        };
        frame.render_widget(header, chunks[0]);
    } else {
        let placeholder = Paragraph::new("No flow loaded")
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(placeholder, chunks[0]);
    }

    // Transcript
    let entries: Vec<_> = state.transcript.entries.clone();
    let transcript_widget = TranscriptWidget {
        entries: &entries,
        scroll_offset: state.transcript.scroll_offset,
        is_live: state.is_running,
        title: "output",
    };
    frame.render_widget(transcript_widget, chunks[1]);

    // Status footer
    let footer_text = if state.is_running {
        format!(" \u{25ba} {}  esc stop", state.status)
    } else {
        " enter continue  q back".to_string()
    };
    let footer = Paragraph::new(footer_text)
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(footer, chunks[2]);
}
