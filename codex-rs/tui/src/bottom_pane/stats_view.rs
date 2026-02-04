//! Stats view for the `/stats` slash command.
//!
//! Displays session statistics in a scrollable popup.

use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use ratatui::buffer::Buffer;
use ratatui::layout::Constraint;
use ratatui::layout::Layout;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Block;
use ratatui::widgets::Widget;
use std::cell::Cell;

use crate::render::Insets;
use crate::render::RectExt as _;
use crate::render::renderable::ColumnRenderable;
use crate::render::renderable::Renderable;
use crate::session_stats::SessionStats;
use crate::session_stats::format_duration;
use crate::status::format_tokens_compact;
use crate::style::user_message_style;

use super::CancellationEvent;
use super::bottom_pane_view::BottomPaneView;
use super::popup_consts::MAX_POPUP_ROWS;
use super::scroll_state::ScrollState;

/// View for displaying session statistics.
pub(crate) struct StatsView {
    lines: Vec<Line<'static>>,
    state: ScrollState,
    complete: bool,
    header: Box<dyn Renderable>,
    last_visible_rows: Cell<usize>,
}

impl StatsView {
    pub(crate) fn new(stats: &SessionStats) -> Self {
        let mut header = ColumnRenderable::new();
        header.push(Line::from("Session Statistics".bold()));
        header.push(Line::from("Performance metrics for this session.".dim()));

        let lines = build_stats_lines(stats);

        let mut view = Self {
            lines,
            state: ScrollState::new(),
            complete: false,
            header: Box::new(header),
            last_visible_rows: Cell::new(MAX_POPUP_ROWS),
        };
        view.state.selected_idx = Some(0);
        view
    }

    fn visible_len(&self) -> usize {
        self.lines.len()
    }

    fn visible_rows_for_scroll(&self) -> usize {
        let len = self.visible_len();
        if len == 0 {
            return 0;
        }
        self.last_visible_rows.get().min(len)
    }

    fn move_up(&mut self) {
        let len = self.visible_len();
        if len == 0 {
            return;
        }
        self.state.move_up_wrap(len);
        self.state
            .ensure_visible(len, self.visible_rows_for_scroll());
    }

    fn move_down(&mut self) {
        let len = self.visible_len();
        if len == 0 {
            return;
        }
        self.state.move_down_wrap(len);
        self.state
            .ensure_visible(len, self.visible_rows_for_scroll());
    }
}

impl BottomPaneView for StatsView {
    fn handle_key_event(&mut self, key_event: KeyEvent) {
        match key_event {
            KeyEvent {
                code: KeyCode::Up, ..
            }
            | KeyEvent {
                code: KeyCode::Char('k'),
                modifiers: KeyModifiers::NONE,
                ..
            } => self.move_up(),
            KeyEvent {
                code: KeyCode::Down,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('j'),
                modifiers: KeyModifiers::NONE,
                ..
            } => self.move_down(),
            KeyEvent {
                code: KeyCode::Esc, ..
            }
            | KeyEvent {
                code: KeyCode::Enter,
                ..
            } => {
                self.on_ctrl_c();
            }
            _ => {}
        }
    }

    fn is_complete(&self) -> bool {
        self.complete
    }

    fn on_ctrl_c(&mut self) -> CancellationEvent {
        self.complete = true;
        CancellationEvent::Handled
    }
}

impl Renderable for StatsView {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 || area.width == 0 {
            return;
        }

        let [content_area, footer_area] =
            Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(area);

        Block::default()
            .style(user_message_style())
            .render(content_area, buf);

        let content_inner = content_area.inset(Insets::vh(1, 2));
        let header_height = self.header.desired_height(content_inner.width);
        let available_height = content_inner.height.saturating_sub(header_height + 1);
        let max_list_height = MAX_POPUP_ROWS.min(self.lines.len()) as u16;
        let list_height = max_list_height.min(available_height);
        let visible_rows = list_height as usize;

        self.last_visible_rows.set(visible_rows);

        let scroll_offset = if visible_rows == 0 {
            0
        } else {
            let mut state = self.state;
            state.ensure_visible(self.lines.len(), visible_rows);
            state.scroll_top
        };
        let visible_lines: Vec<_> = self
            .lines
            .iter()
            .skip(scroll_offset)
            .take(visible_rows)
            .cloned()
            .collect();

        let [header_area, _, list_area] = Layout::vertical([
            Constraint::Max(header_height),
            Constraint::Max(1),
            Constraint::Length(list_height),
        ])
        .areas(content_inner);

        self.header.render(header_area, buf);

        // Render the lines
        for (i, line) in visible_lines.iter().enumerate() {
            let y = list_area.y + i as u16;
            if y >= list_area.y + list_area.height {
                break;
            }
            let line_area = Rect {
                x: list_area.x,
                y,
                width: list_area.width,
                height: 1,
            };
            line.clone().render(line_area, buf);
        }

        // Footer hint
        let hint = Line::from(vec![
            "Press ".into(),
            "↑/↓".cyan(),
            " to scroll, ".into(),
            "Esc".cyan(),
            " to close".into(),
        ]);
        let hint_area = Rect {
            x: footer_area.x + 2,
            y: footer_area.y,
            width: footer_area.width.saturating_sub(2),
            height: footer_area.height,
        };
        hint.dim().render(hint_area, buf);
    }

    fn desired_height(&self, width: u16) -> u16 {
        let header_height = self.header.desired_height(width.saturating_sub(4));
        let lines_height = MAX_POPUP_ROWS.min(self.lines.len()) as u16;

        // header + gap + lines + footer + padding
        header_height + 1 + lines_height + 1 + 2
    }
}

/// Build the display lines for the stats view.
fn build_stats_lines(stats: &SessionStats) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    // Section: Commands
    lines.push(section_header("Commands"));
    lines.push(stat_line(
        "Total executed",
        &stats.total_commands().to_string(),
    ));
    lines.push(stat_line(
        "Successful",
        &format!(
            "{} ({:.1}%)",
            stats.successful_commands(),
            stats.success_rate()
        ),
    ));
    lines.push(stat_line("Failed", &stats.failed_commands().to_string()));
    lines.push(stat_line(
        "Total exec time",
        &format_duration(stats.total_command_time()),
    ));
    lines.push(Line::from(""));

    // Section: Files
    lines.push(section_header("Files"));
    lines.push(stat_line(
        "Files modified",
        &stats.files_modified_count().to_string(),
    ));
    lines.push(stat_line(
        "Files accessed",
        &stats.files_accessed_count().to_string(),
    ));

    let top_accessed = stats.top_accessed_files(3);
    if !top_accessed.is_empty() {
        lines.push(Line::from(vec![Span::from("  Top accessed:").dim()]));
        for (path, count) in top_accessed {
            let filename = path
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| path.display().to_string());
            lines.push(Line::from(vec![
                Span::from("    "),
                Span::from(filename),
                Span::from(format!(" ({count}x)")).dim(),
            ]));
        }
    }
    lines.push(Line::from(""));

    // Section: Turns & Tokens
    lines.push(section_header("Turns & Tokens"));
    lines.push(stat_line("Total turns", &stats.current_turn().to_string()));
    lines.push(stat_line(
        "Total tokens",
        &format_tokens_compact(stats.total_tokens()),
    ));
    lines.push(stat_line(
        "Input tokens",
        &format_tokens_compact(stats.total_input_tokens()),
    ));
    lines.push(stat_line(
        "Output tokens",
        &format_tokens_compact(stats.total_output_tokens()),
    ));

    // Token breakdown by turn (show last 5 turns if available)
    let turn_breakdown = stats.turn_token_breakdown();
    if !turn_breakdown.is_empty() {
        lines.push(Line::from(vec![Span::from("  Recent turns:").dim()]));
        for turn in turn_breakdown.iter().rev().take(5).rev() {
            lines.push(Line::from(vec![
                Span::from(format!("    Turn {}: ", turn.turn_number)),
                Span::from(format_tokens_compact(turn.input_tokens)).dim(),
                Span::from(" in, ").dim(),
                Span::from(format_tokens_compact(turn.output_tokens)).dim(),
                Span::from(" out").dim(),
            ]));
        }
    }
    lines.push(Line::from(""));

    // Section: Timing
    lines.push(section_header("Timing"));
    lines.push(stat_line(
        "Session duration",
        &format_duration(stats.session_duration()),
    ));
    lines.push(stat_line(
        "Model wait time",
        &format!(
            "{} ({:.1}%)",
            format_duration(stats.model_wait_time()),
            stats.model_wait_percentage()
        ),
    ));
    lines.push(stat_line(
        "Tool exec time",
        &format!(
            "{} ({:.1}%)",
            format_duration(stats.tool_execution_time()),
            stats.tool_execution_percentage()
        ),
    ));

    lines
}

/// Create a section header line.
fn section_header(title: &str) -> Line<'static> {
    Line::from(vec![
        Span::from("── ").dim(),
        Span::from(title.to_string()).bold(),
        Span::from(" ──").dim(),
    ])
}

/// Create a stat line with label and value.
fn stat_line(label: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::from("  "),
        Span::from(format!("{label}: ")),
        Span::from(value.to_string()).cyan(),
    ])
}
