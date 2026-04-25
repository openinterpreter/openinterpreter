use crate::color::blend;
use crate::color::is_light;
use crate::terminal_palette::StdoutColorLevel;
use crate::terminal_palette::best_color;
use crate::terminal_palette::default_bg;
use crate::terminal_palette::rgb_color;
use crate::terminal_palette::stdout_color_level;
use ratatui::prelude::Stylize;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;

pub const APP_ACCENT_DARK_RGB: (u8, u8, u8) = (236, 236, 236);
pub const APP_ACCENT_LIGHT_RGB: (u8, u8, u8) = (28, 28, 28);
pub const APP_SURFACE_DARK_RGB: (u8, u8, u8) = (16, 16, 16);

pub fn app_accent_color() -> Color {
    let accent_rgb = match default_bg() {
        Some(bg) if is_light(bg) => APP_ACCENT_LIGHT_RGB,
        Some(_) | None => APP_ACCENT_DARK_RGB,
    };
    match stdout_color_level() {
        StdoutColorLevel::Unknown => rgb_color(accent_rgb),
        _ => best_color(accent_rgb),
    }
}

pub fn app_accent_style() -> Style {
    Style::default().fg(app_accent_color())
}

pub fn app_accent_underlined_style() -> Style {
    app_accent_style().add_modifier(Modifier::UNDERLINED)
}

pub fn selected_option_style() -> Style {
    Style::default().fg(app_accent_color()).bold()
}

pub fn unselected_option_style() -> Style {
    Style::default().dim()
}

pub fn composer_style() -> Style {
    app_surface_style_for(default_bg())
}

pub fn user_message_style() -> Style {
    app_surface_style_for(default_bg())
}

pub fn proposed_plan_style() -> Style {
    proposed_plan_style_for(default_bg())
}

pub fn composer_style_for(terminal_bg: Option<(u8, u8, u8)>) -> Style {
    app_surface_style_for(terminal_bg)
}

/// Returns the style for a user-authored message using the provided terminal background.
pub fn user_message_style_for(terminal_bg: Option<(u8, u8, u8)>) -> Style {
    app_surface_style_for(terminal_bg)
}

pub fn app_surface_style() -> Style {
    app_surface_style_for(default_bg())
}

pub fn app_surface_style_for(terminal_bg: Option<(u8, u8, u8)>) -> Style {
    Style::default().bg(app_surface_bg_for(terminal_bg))
}

pub fn proposed_plan_style_for(terminal_bg: Option<(u8, u8, u8)>) -> Style {
    app_surface_style_for(terminal_bg)
}

#[allow(clippy::disallowed_methods)]
pub fn app_surface_bg() -> Color {
    app_surface_bg_for(default_bg())
}

#[allow(clippy::disallowed_methods)]
pub fn app_surface_bg_for(terminal_bg: Option<(u8, u8, u8)>) -> Color {
    match terminal_bg {
        Some(bg) if is_light(bg) => rgb_color(blend((0, 0, 0), bg, 0.04)),
        Some(_) | None => rgb_color(APP_SURFACE_DARK_RGB),
    }
}

#[allow(clippy::disallowed_methods)]
pub fn composer_bg(terminal_bg: (u8, u8, u8)) -> Color {
    app_surface_bg_for(Some(terminal_bg))
}

#[allow(clippy::disallowed_methods)]
pub fn user_message_bg(terminal_bg: (u8, u8, u8)) -> Color {
    app_surface_bg_for(Some(terminal_bg))
}

#[allow(clippy::disallowed_methods)]
pub fn proposed_plan_bg(terminal_bg: (u8, u8, u8)) -> Color {
    app_surface_bg_for(Some(terminal_bg))
}

#[cfg(test)]
mod tests {
    use super::composer_style_for;
    use super::user_message_style_for;
    use insta::assert_debug_snapshot;

    #[test]
    fn composer_style_uses_nearly_black_bg_in_dark_mode() {
        assert_debug_snapshot!(composer_style_for(Some((0, 0, 0))));
    }

    #[test]
    fn user_message_style_matches_composer_surface_in_dark_mode() {
        assert_debug_snapshot!(user_message_style_for(Some((0, 0, 0))));
    }
}
