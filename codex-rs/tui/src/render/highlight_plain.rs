//! Plain-text fallback for thin-client builds without syntax highlighting.

use ratatui::text::Line;
use ratatui::text::Span;
use std::path::Path;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::RwLock;

static THEME: OnceLock<RwLock<Theme>> = OnceLock::new();
static THEME_OVERRIDE: OnceLock<Option<String>> = OnceLock::new();
static CODEX_HOME: OnceLock<Option<PathBuf>> = OnceLock::new();

const MAX_HIGHLIGHT_BYTES: usize = 512 * 1024;
const MAX_HIGHLIGHT_LINES: usize = 10_000;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct Theme {
    name: String,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct DiffScopeBackgroundRgbs {
    pub inserted: Option<(u8, u8, u8)>,
    pub deleted: Option<(u8, u8, u8)>,
}

pub(crate) struct ThemeEntry {
    pub name: String,
    pub is_custom: bool,
}

pub(crate) fn set_theme_override(
    name: Option<String>,
    codex_home: Option<PathBuf>,
) -> Option<String> {
    let resolved = resolve_theme_with_override(name.as_deref(), codex_home.as_deref());
    let _ = THEME_OVERRIDE.set(name);
    let _ = CODEX_HOME.set(codex_home);
    if THEME.get().is_some() {
        set_syntax_theme(resolved);
    }
    None
}

#[allow(dead_code)]
pub(crate) fn validate_theme_name(name: Option<&str>, codex_home: Option<&Path>) -> Option<String> {
    let name = name?;
    if theme_exists(name, codex_home) {
        return None;
    }
    let custom_theme_path_display = codex_home
        .map(|home| custom_theme_path(name, home).display().to_string())
        .unwrap_or_else(|| format!("$CODEX_HOME/themes/{name}.tmTheme"));
    Some(format!(
        "Theme \"{name}\" not found. Using the default theme. \
         To use a custom theme, place a .tmTheme file at \
         {custom_theme_path_display}."
    ))
}

pub(crate) fn adaptive_default_theme_name() -> &'static str {
    match crate::terminal_palette::default_bg() {
        Some(bg) if crate::color::is_light(bg) => "catppuccin-latte",
        _ => "catppuccin-mocha",
    }
}

pub(crate) fn set_syntax_theme(theme: Theme) {
    let mut guard = match theme_lock().write() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    *guard = theme;
}

pub(crate) fn current_syntax_theme() -> Theme {
    match theme_lock().read() {
        Ok(theme) => theme.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

pub(crate) fn diff_scope_background_rgbs() -> DiffScopeBackgroundRgbs {
    DiffScopeBackgroundRgbs::default()
}

pub(crate) fn configured_theme_name() -> String {
    if let Some(Some(name)) = THEME_OVERRIDE.get()
        && theme_exists(name, CODEX_HOME.get().and_then(|home| home.as_deref()))
    {
        return name.clone();
    }
    adaptive_default_theme_name().to_string()
}

pub(crate) fn resolve_theme_by_name(name: &str, codex_home: Option<&Path>) -> Option<Theme> {
    theme_exists(name, codex_home).then(|| Theme {
        name: name.to_string(),
    })
}

pub(crate) fn list_available_themes(codex_home: Option<&Path>) -> Vec<ThemeEntry> {
    let mut entries: Vec<ThemeEntry> = BUILTIN_THEME_NAMES
        .iter()
        .map(|name| ThemeEntry {
            name: (*name).to_string(),
            is_custom: false,
        })
        .collect();

    if let Some(home) = codex_home
        && let Ok(read_dir) = std::fs::read_dir(home.join("themes"))
    {
        for entry in read_dir.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) == Some("tmTheme")
                && let Some(stem) = path.file_stem().and_then(|stem| stem.to_str())
            {
                let name = stem.to_string();
                if !entries.iter().any(|entry| entry.name == name) {
                    entries.push(ThemeEntry {
                        name,
                        is_custom: true,
                    });
                }
            }
        }
    }

    entries.sort_by_cached_key(|entry| (entry.name.to_ascii_lowercase(), entry.name.clone()));
    entries
}

pub(crate) fn exceeds_highlight_limits(total_bytes: usize, total_lines: usize) -> bool {
    total_bytes > MAX_HIGHLIGHT_BYTES || total_lines > MAX_HIGHLIGHT_LINES
}

pub(crate) fn highlight_code_to_lines(code: &str, _lang: &str) -> Vec<Line<'static>> {
    let mut result: Vec<Line<'static>> = code
        .lines()
        .map(|line| Line::from(line.to_string()))
        .collect();
    if result.is_empty() {
        result.push(Line::from(String::new()));
    }
    result
}

pub(crate) fn highlight_bash_to_lines(script: &str) -> Vec<Line<'static>> {
    highlight_code_to_lines(script, "bash")
}

pub(crate) fn highlight_code_to_styled_spans(
    code: &str,
    _lang: &str,
) -> Option<Vec<Vec<Span<'static>>>> {
    if code.is_empty() || exceeds_highlight_limits(code.len(), code.lines().count()) {
        return None;
    }
    None
}

const BUILTIN_THEME_NAMES: &[&str] = &[
    "1337",
    "ansi",
    "base16",
    "base16-256",
    "base16-eighties-dark",
    "base16-mocha-dark",
    "base16-ocean-dark",
    "base16-ocean-light",
    "catppuccin-frappe",
    "catppuccin-latte",
    "catppuccin-macchiato",
    "catppuccin-mocha",
    "coldark-cold",
    "coldark-dark",
    "dark-neon",
    "dracula",
    "github",
    "gruvbox-dark",
    "gruvbox-light",
    "inspired-github",
    "monokai-extended",
    "monokai-extended-bright",
    "monokai-extended-light",
    "monokai-extended-origin",
    "nord",
    "one-half-dark",
    "one-half-light",
    "solarized-dark",
    "solarized-light",
    "sublime-snazzy",
    "two-dark",
    "zenburn",
];

fn theme_lock() -> &'static RwLock<Theme> {
    THEME.get_or_init(|| RwLock::new(build_default_theme()))
}

fn build_default_theme() -> Theme {
    let name = THEME_OVERRIDE.get().and_then(|name| name.as_deref());
    let codex_home = CODEX_HOME.get().and_then(|home| home.as_deref());
    resolve_theme_with_override(name, codex_home)
}

fn resolve_theme_with_override(name: Option<&str>, codex_home: Option<&Path>) -> Theme {
    if let Some(name) = name
        && theme_exists(name, codex_home)
    {
        return Theme {
            name: name.to_string(),
        };
    }
    Theme {
        name: adaptive_default_theme_name().to_string(),
    }
}

fn custom_theme_path(name: &str, codex_home: &Path) -> PathBuf {
    codex_home.join("themes").join(format!("{name}.tmTheme"))
}

fn theme_exists(name: &str, codex_home: Option<&Path>) -> bool {
    BUILTIN_THEME_NAMES.contains(&name)
        || codex_home
            .map(|home| custom_theme_path(name, home).is_file())
            .unwrap_or(false)
}
