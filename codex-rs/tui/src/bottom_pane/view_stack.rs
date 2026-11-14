//! Shared centered-dialog geometry for retained view stacks and full transcript surfaces.
//! Only the top view receives input; painting a backdrop never changes focus.

use super::BottomPaneView;
use super::ViewPresentation;
use crate::render::renderable::Renderable;
use crate::render::renderable::RenderableItem;
use crossterm::cursor::SetCursorStyle;
use ratatui::buffer::Buffer;
use ratatui::buffer::CellDiffOption;
use ratatui::layout::Rect;
use ratatui::widgets::Clear;
use ratatui::widgets::Widget;

pub(super) struct ViewStack<'a>(pub(super) &'a [Box<dyn BottomPaneView>]);

fn dialog_area(view: &dyn BottomPaneView, area: Rect) -> Rect {
    let width = area
        .width
        .saturating_sub(/*rhs*/ 4)
        .max(area.width.min(/*other*/ 8))
        .min(/*other*/ 72);
    let height = view.desired_height(width).min(area.height);
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}

impl Renderable for ViewStack<'_> {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        let Some((view, previous)) = self.0.split_last() else {
            return;
        };
        if view.presentation() == ViewPresentation::Centered {
            Self(previous).render(area, buf);
            CenteredView(view.as_ref()).render(area, buf);
        } else {
            view.render(area, buf);
        }
    }

    fn desired_height(&self, width: u16) -> u16 {
        let Some((view, previous)) = self.0.split_last() else {
            return 0;
        };
        if view.presentation() == ViewPresentation::Centered {
            CenteredView(view.as_ref())
                .desired_height(width)
                .max(Self(previous).desired_height(width))
        } else {
            view.desired_height(width)
        }
    }

    fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        let view = self.0.last()?;
        view.cursor_pos(dialog_area(view.as_ref(), area))
    }

    fn cursor_style(&self, area: Rect) -> SetCursorStyle {
        self.0
            .last()
            .map_or(SetCursorStyle::DefaultUserShape, |view| {
                view.cursor_style(dialog_area(view.as_ref(), area))
            })
    }
}

/// A dialog placed within the supplied viewport, without taking over its backdrop.
pub(crate) struct CenteredView<'a>(pub(crate) &'a dyn BottomPaneView);

impl CenteredView<'_> {
    pub(crate) fn area(&self, viewport: Rect) -> Rect {
        dialog_area(self.0, viewport)
    }
}

impl Renderable for CenteredView<'_> {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        let dialog = self.area(area);
        Clear.render(dialog, buf);
        // A dialog can cover cached blank cells whose color the terminal did not retain.
        // Paint its whole surface; let wide text rendering still reset continuation cells.
        for y in dialog.top()..dialog.bottom() {
            for x in dialog.left()..dialog.right() {
                buf[(x, y)].set_diff_option(CellDiffOption::AlwaysUpdate);
            }
        }
        self.0.render(dialog, buf);
    }

    fn desired_height(&self, width: u16) -> u16 {
        self.area(Rect::new(/*x*/ 0, /*y*/ 0, width, u16::MAX))
            .height
    }

    fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        self.0.cursor_pos(self.area(area))
    }

    fn cursor_style(&self, area: Rect) -> SetCursorStyle {
        self.0.cursor_style(self.area(area))
    }
}

pub(crate) struct DialogOverlay<'a> {
    pub(crate) backdrop: RenderableItem<'a>,
    pub(crate) dialog: CenteredView<'a>,
}

impl Renderable for DialogOverlay<'_> {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        self.backdrop.render(area, buf);
        self.dialog.render(area, buf);
    }

    fn desired_height(&self, width: u16) -> u16 {
        self.backdrop
            .desired_height(width)
            .max(self.dialog.desired_height(width))
    }

    fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        self.dialog.cursor_pos(area)
    }

    fn cursor_style(&self, area: Rect) -> SetCursorStyle {
        self.dialog.cursor_style(area)
    }
}

#[cfg(test)]
#[path = "view_stack_tests.rs"]
mod tests;
