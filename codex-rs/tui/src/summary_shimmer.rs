//! Smooth, whole-grapheme status shimmer with a two-second sweep.
//!
//! Only brightness changes: the moving band uses the terminal foreground, while
//! the remaining text blends halfway into the background. The wave spans at
//! least six terminal columns so short labels do not flash one letter at a time.
//! Unknown palettes use static dim text instead of a stepped animation.

use std::time::Duration;

use ratatui::style::Style;
use ratatui::text::Span;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::color::blend;
use crate::motion::MotionMode;
use crate::terminal_palette::StdoutColorLevel;
use crate::terminal_palette::default_bg;
use crate::terminal_palette::default_fg;
use crate::terminal_palette::effective_stdout_color_level;
use crate::terminal_palette::rgb_color;

pub(crate) fn summary_shimmer(
    text: &str,
    elapsed: Duration,
    motion: MotionMode,
) -> Vec<Span<'static>> {
    if motion == MotionMode::Reduced {
        return vec![text.to_owned().into()];
    }
    let (StdoutColorLevel::TrueColor, Some(fg), Some(bg)) =
        (effective_stdout_color_level(), default_fg(), default_bg())
    else {
        return vec![Span::styled(text.to_owned(), Style::default().dim())];
    };
    let width = text.width() as f64;
    let half_width = (width * 0.1).max(/*other*/ 3.0);
    let position = (elapsed.as_secs_f64() % 2.0) / 2.0 * (width + 2.0 * half_width) - half_width;
    let mut column = 0.0;
    text.graphemes(/*is_extended*/ true)
        .map(|grapheme| {
            let glyph_width = grapheme.width() as f64;
            let center = column + glyph_width / 2.0;
            column += glyph_width;
            let distance = ((center - position).abs() / half_width).min(/*other*/ 1.0);
            let intensity = 0.5 * (1.0 + (std::f64::consts::PI * distance).cos());
            let alpha = (0.5 + 0.5 * intensity) as f32;
            let style = Style::default().fg(rgb_color(blend(fg, bg, alpha)));
            Span::styled(grapheme.to_owned(), style)
        })
        .collect()
}

#[cfg(test)]
#[path = "summary_shimmer_tests.rs"]
mod tests;
