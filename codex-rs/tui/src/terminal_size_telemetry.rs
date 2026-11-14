//! Shared terminal-cell buckets and best-effort observations using the existing metrics provider.

use crate::legacy_core::config::Config;
use codex_config::types::OtelExporterKind;
use ratatui::layout::Size;

pub(crate) type LaunchTags = Vec<(&'static str, &'static str)>;

pub(crate) fn enabled(config: &Config) -> bool {
    match config.otel.metrics_exporter {
        OtelExporterKind::None => false,
        OtelExporterKind::Statsig => config.analytics_enabled != Some(false),
        OtelExporterKind::OtlpHttp { .. } | OtelExporterKind::OtlpGrpc { .. } => true,
    }
}

pub(crate) fn tags(size: Size) -> [(&'static str, &'static str); 2] {
    // Keep values compatible with metrics tag validation; `<` and `+` are rejected.
    let columns = match size.width {
        0 => "unknown",
        1..40 => "under-40",
        40..80 => "40-79",
        80..120 => "80-119",
        120..240 => "120-239",
        240..480 => "240-479",
        480.. => "480-and-up",
    };
    let rows = match size.height {
        0 => "unknown",
        1..15 => "under-15",
        15..24 => "15-23",
        24..40 => "24-39",
        40..80 => "40-79",
        80..120 => "80-119",
        120.. => "120-and-up",
    };
    [("terminal_columns", columns), ("terminal_rows", rows)]
}

pub(crate) fn read_size() -> Size {
    // On Unix, size() may synchronously spawn tput when the direct query fails.
    #[cfg(unix)]
    let size = crossterm::terminal::window_size().map(|size| Size::new(size.columns, size.rows));
    // Windows size() queries the console directly; window_size() is unsupported there.
    #[cfg(not(unix))]
    let size = crossterm::terminal::size().map(|(width, height)| Size::new(width, height));
    size.unwrap_or_default()
}

pub(crate) fn record(context: &LaunchTags, config: &Config, reason: &'static str, size: Size) {
    if !context.is_empty()
        && let Some(metrics) = codex_otel::global()
    {
        let mut labels = context.clone();
        labels.extend(tags(size));
        labels.push(("observation_reason", reason));
        let _ = metrics.counter_with_analytics_consent(
            "codex.tui.terminal_size",
            /*inc*/ 1,
            &labels,
            config.analytics_enabled != Some(false),
        );
    }
}
