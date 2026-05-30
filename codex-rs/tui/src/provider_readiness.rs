//! Provider readiness labels for the `/model` picker.
//!
//! The implementation now lives in
//! `codex_model_provider_info::provider_selection` so the TUI and the
//! app-server compute identical readiness. These re-exports preserve the
//! existing `crate::provider_readiness::*` import paths.

pub(crate) use codex_model_provider_info::provider_selection::ProviderReadinessSnapshot;
pub(crate) use codex_model_provider_info::provider_selection::readiness_for_configured_provider;
pub(crate) use codex_model_provider_info::provider_selection::readiness_for_provider_preset;
