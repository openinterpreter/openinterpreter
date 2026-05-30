//! Provider choices for the `/model` provider picker.
//!
//! The implementation now lives in
//! `codex_model_provider_info::provider_selection` so the TUI and the
//! app-server build identical provider lists. These re-exports preserve the
//! existing `crate::provider_model_flow::*` import paths; call sites pass
//! `&config.model_providers`, `&config.model_provider_id`, and (for the
//! snapshot-building entry point) `config.codex_home.as_path()`.

pub(crate) use codex_model_provider_info::provider_selection::ProviderChoice;
pub(crate) use codex_model_provider_info::provider_selection::ProviderChoiceAction;
pub(crate) use codex_model_provider_info::provider_selection::model_picker_provider_choices;
// Only the test-only snapshot-injection helper in `chatwidget/model_selection.rs`
// uses this entry point, so gate the re-export to match its sole consumer.
#[cfg(test)]
pub(crate) use codex_model_provider_info::provider_selection::model_picker_provider_choices_with_snapshot;
pub(crate) use codex_model_provider_info::provider_selection::provider_choice_description;
pub(crate) use codex_model_provider_info::provider_selection::provider_preset_choice_description;
