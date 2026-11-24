//! Metrics derived from loaded configuration at session start.

use super::Config;
use codex_config::ConfigLayerSource;
use codex_config::ConfigLayerStack;
use codex_otel::SessionTelemetry;

/// Retain attribution for a legacy multi-agent opt-out even when a later
/// session override enables collaboration for only this turn.
pub(crate) fn legacy_multi_agent_disabled_sources(stack: &ConfigLayerStack) -> Vec<&'static str> {
    stack
        .layers_low_to_high()
        .filter(|layer| {
            layer
                .config
                .get("features")
                .and_then(|features| features.get("multi_agent"))
                .and_then(toml::Value::as_bool)
                == Some(false)
        })
        .map(|layer| match &layer.name {
            ConfigLayerSource::User { .. } => "user",
            ConfigLayerSource::System { .. } => "system",
            ConfigLayerSource::Project { .. } => "project",
            ConfigLayerSource::SessionFlags => "session",
            ConfigLayerSource::PackagedDefaults { .. } => "packaged",
            ConfigLayerSource::EnterpriseManaged { .. } => "enterprise",
            ConfigLayerSource::Mdm { .. } | ConfigLayerSource::LegacyManagedConfigTomlFromMdm => {
                "mdm"
            }
            ConfigLayerSource::LegacyManagedConfigTomlFromFile { .. } => "managed_file",
        })
        .collect()
}

pub(crate) fn emit_session_start_metrics(config: &Config, telemetry: &SessionTelemetry) {
    // Registry keys are static and bounded; never use arbitrary config values as metric labels.
    for feature in config.features.enabled_features() {
        telemetry.counter("codex.feature.enabled", 1, &[("feature", feature.key())]);
    }
    #[cfg(windows)]
    crate::windows_system_config::emit_namespace_squatting_probe();
}
