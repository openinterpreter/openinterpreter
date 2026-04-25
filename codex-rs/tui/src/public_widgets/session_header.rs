use crate::history_cell::HistoryCell;
use crate::history_cell::SessionHeaderHistoryCell;
use crate::version::CODEX_CLI_VERSION;
use codex_protocol::openai_models::ReasoningEffort as ReasoningEffortConfig;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Widget;
use std::path::PathBuf;

/// Public wrapper for the session header card used by the TUI.
pub struct SessionHeaderCard {
    inner: SessionHeaderHistoryCell,
}

impl SessionHeaderCard {
    pub fn new(
        model: String,
        reasoning_effort: Option<ReasoningEffortConfig>,
        show_fast_status: bool,
        directory: PathBuf,
    ) -> Self {
        Self {
            inner: SessionHeaderHistoryCell::new(
                model,
                reasoning_effort,
                show_fast_status,
                directory,
                CODEX_CLI_VERSION,
            ),
        }
    }

    pub fn pending(model: String, directory: PathBuf) -> Self {
        Self::pending_with_reasoning(model, /*reasoning_effort*/ None, directory)
    }

    pub fn pending_with_reasoning(
        model: String,
        reasoning_effort: Option<ReasoningEffortConfig>,
        directory: PathBuf,
    ) -> Self {
        Self {
            inner: SessionHeaderHistoryCell::new_with_style(
                model,
                Style::default().add_modifier(Modifier::DIM | Modifier::ITALIC),
                reasoning_effort,
                /*show_fast_status*/ false,
                directory,
                CODEX_CLI_VERSION,
            ),
        }
    }

    pub fn lines(&self, width: u16) -> Vec<Line<'static>> {
        self.inner.display_lines(width)
    }

    pub fn height_for_width(&self, width: u16) -> u16 {
        self.lines(width).len() as u16
    }

    pub fn render_ref(&self, area: Rect, buf: &mut Buffer) {
        Paragraph::new(self.lines(area.width)).render(area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn pending_session_header_snapshot() {
        let card = SessionHeaderCard::pending("loading".to_string(), PathBuf::from("/tmp/project"));
        let rendered = card
            .lines(/*width*/ 54)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn pending_header_height_matches_lines() {
        let card = SessionHeaderCard::pending("loading".to_string(), PathBuf::from("/tmp/project"));
        assert_eq!(card.height_for_width(54), card.lines(54).len() as u16);
    }
}
