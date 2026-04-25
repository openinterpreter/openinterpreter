use crossterm::event::KeyEvent;
use ratatui::buffer::Buffer;
use ratatui::prelude::Widget;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use ratatui::widgets::WidgetRef;
use ratatui::widgets::Wrap;

use crate::onboarding::onboarding_screen::KeyboardHandler;
use crate::onboarding::onboarding_screen::StepStateProvider;
use crate::product_branding::ProductBranding;
use crate::tui::FrameRequester;

use super::onboarding_screen::StepState;

pub(crate) struct WelcomeWidget {
    pub is_logged_in: bool,
    branding: ProductBranding,
}

impl KeyboardHandler for WelcomeWidget {
    fn handle_key_event(&mut self, _key_event: KeyEvent) {}
}

impl WelcomeWidget {
    pub(crate) fn new(
        is_logged_in: bool,
        _request_frame: FrameRequester,
        _animations_enabled: bool,
    ) -> Self {
        Self {
            is_logged_in,
            branding: ProductBranding::current(),
        }
    }

    pub(crate) fn update_layout_area(&self, _area: ratatui::layout::Rect) {}
}

impl WidgetRef for &WelcomeWidget {
    fn render_ref(&self, area: ratatui::layout::Rect, buf: &mut Buffer) {
        let lines = vec![
            "".into(),
            Line::from(vec![
                "  ".into(),
                "Welcome to ".into(),
                self.branding.display_name.bold(),
                self.branding.welcome_suffix.into(),
            ]),
        ];

        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render(area, buf);
    }
}

impl StepStateProvider for WelcomeWidget {
    fn get_step_state(&self) -> StepState {
        match self.is_logged_in {
            true => StepState::Hidden,
            false => StepState::Complete,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use ratatui::Terminal;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    use crate::test_backend::VT100Backend;

    fn row_containing(buf: &Buffer, needle: &str) -> Option<u16> {
        (0..buf.area.height).find(|&y| {
            let mut row = String::new();
            for x in 0..buf.area.width {
                row.push_str(buf[(x, y)].symbol());
            }
            row.contains(needle)
        })
    }

    #[test]
    fn welcome_renders_with_one_blank_row_above() {
        let widget = WelcomeWidget::new(
            /*is_logged_in*/ false,
            FrameRequester::test_dummy(),
            /*animations_enabled*/ true,
        );
        let area = Rect::new(0, 0, /*width*/ 70, /*height*/ 6);
        let mut buf = Buffer::empty(area);
        (&widget).render(area, &mut buf);

        let welcome_row = row_containing(&buf, "Welcome");
        assert_eq!(welcome_row, Some(1));
    }

    #[test]
    fn welcome_renders_without_animation_when_enabled() {
        let widget = WelcomeWidget::new(
            /*is_logged_in*/ false,
            FrameRequester::test_dummy(),
            /*animations_enabled*/ true,
        );
        let area = Rect::new(0, 0, /*width*/ 70, /*height*/ 6);
        let mut buf = Buffer::empty(area);
        (&widget).render(area, &mut buf);

        let welcome_row = row_containing(&buf, "Welcome");
        assert_eq!(welcome_row, Some(1));
    }

    #[test]
    fn renders_open_interpreter_snapshot() {
        let widget = WelcomeWidget {
            is_logged_in: false,
            branding: ProductBranding::for_open_interpreter(/*is_open_interpreter*/ true),
        };

        let mut terminal =
            Terminal::new(VT100Backend::new(/*width*/ 70, /*height*/ 6)).expect("terminal");
        terminal
            .draw(|f| (&widget).render_ref(f.area(), f.buffer_mut()))
            .expect("draw");

        insta::assert_snapshot!(terminal.backend());
    }
}
