//! Bounded observations for foreground callers. This module never exports telemetry;
//! in particular, the detached updater must not create a telemetry provider.

use std::fmt;
use std::path::Path;

/// Machine-readable startup classification, without changing the caller's error context.
#[derive(Debug)]
pub enum StartFailure {
    Spawn(String),
    EarlyChildExit(String),
    ReadinessTimeout(String),
}

impl fmt::Display for StartFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn(message)
            | Self::EarlyChildExit(message)
            | Self::ReadinessTimeout(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for StartFailure {}

/// Preserve the error chain so a consumer can classify a startup failure
/// without parsing human-readable stderr or treating unknown failures as success.
pub struct StartError(anyhow::Error);

impl From<anyhow::Error> for StartError {
    fn from(error: anyhow::Error) -> Self {
        Self(error)
    }
}

impl StartError {
    pub fn failure_reason(&self) -> &'static str {
        match self
            .0
            .chain()
            .find_map(|error| error.downcast_ref::<StartFailure>())
        {
            Some(StartFailure::Spawn(_)) => "spawn",
            Some(StartFailure::EarlyChildExit(_)) => "early_child_exit",
            Some(StartFailure::ReadinessTimeout(_)) => "readiness_timeout",
            None => "unknown",
        }
    }
}

/// Suppress child reporting when the foreground TUI handoff owns the observation.
pub const HANDOFF_ENV: &str = "CODEX_DAEMON_TELEMETRY_HANDOFF";

/// Only presence and enabled state leave the settings reader, never values or contents.
/// Missing files mean defaults; unreadable or invalid settings remain unknown.
pub async fn settings_tags(codex_home: &Path) -> [(&'static str, &'static str); 4] {
    let path = codex_home
        .join(crate::STATE_DIR_NAME)
        .join(crate::SETTINGS_FILE_NAME);
    crate::settings::telemetry_tags(&path).await.unwrap_or([
        ("auto_update", "unknown"),
        ("auto_update_setting", "unknown"),
        ("update_interval_setting", "unknown"),
        ("shutdown_grace_setting", "unknown"),
    ])
}
