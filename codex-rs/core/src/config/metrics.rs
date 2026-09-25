//! Metrics derived from loaded configuration at session start.

use super::Config;
use codex_otel::SessionTelemetry;

pub(crate) fn emit_session_start_metrics(config: &Config, telemetry: &SessionTelemetry) {
    config.features.emit_metrics(telemetry);
    #[cfg(windows)]
    crate::windows_system_config::emit_namespace_squatting_probe();
}
