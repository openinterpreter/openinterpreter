use std::collections::HashSet;

use codex_model_provider_info::BundledProviderCatalogEntry;
use codex_model_provider_info::ModelProviderInfo;
use codex_model_provider_info::OPENAI_PROVIDER_ID;
#[cfg(test)]
use codex_model_provider_info::WireApi;
use codex_model_provider_info::bundled_provider_catalog;
use codex_model_provider_info::harness_selection::HarnessChoice;
use codex_model_provider_info::harness_selection::harness_choices_for_provider_model;
use codex_protocol::openai_models::ModelPreset;
use codex_protocol::openai_models::ReasoningEffort as ReasoningEffortConfig;
use strum::IntoEnumIterator;

use crate::onboarding::local_provider::can_start_local_provider;
use crate::onboarding::local_provider::is_local_provider;
use crate::onboarding::local_provider::is_local_provider_running;
use crate::onboarding::local_provider::no_models_message;
use crate::onboarding::local_provider::not_running_message;

const OPENAI_API_KEY_PROVIDER_ID: &str = "openai_api_key";
const OPENAI_PICKER_PRIORITY_MODELS: [&str; 4] =
    ["gpt-5.4", "gpt-5.4-mini", "gpt-5.4-nano", "gpt-5.3-codex"];

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProviderHarnessSelectionState {
    pub(crate) provider_id: String,
    pub(crate) provider_name: String,
    pub(crate) model: String,
    pub(crate) effort: Option<ReasoningEffortConfig>,
    choices: Vec<HarnessChoice>,
    selected_idx: usize,
}

impl ProviderHarnessSelectionState {
    pub(crate) fn new(
        provider_id: String,
        provider_name: String,
        provider: Option<&ModelProviderInfo>,
        model: String,
        effort: Option<ReasoningEffortConfig>,
    ) -> Self {
        let choices = harness_choices_for_provider_model(
            provider_id.as_str(),
            Some(provider_name.as_str()),
            provider.and_then(|provider| provider.base_url.as_deref()),
            provider.map(|provider| provider.wire_api),
            Some(model.as_str()),
        );
        Self {
            provider_id,
            provider_name,
            model,
            effort,
            choices,
            selected_idx: 0,
        }
    }

    pub(crate) fn choices(&self) -> &[HarnessChoice] {
        &self.choices
    }

    pub(crate) fn selected_idx(&self) -> usize {
        self.selected_idx
    }

    pub(crate) fn selected_harness(&self) -> Option<String> {
        self.choices
            .get(self.selected_idx)
            .and_then(|choice| choice.stored.clone())
    }

    pub(crate) fn move_selection(&mut self, delta: isize) {
        if self.choices.is_empty() {
            return;
        }
        let len = self.choices.len() as isize;
        self.selected_idx = (self.selected_idx as isize + delta).rem_euclid(len) as usize;
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LoadingProviderModelsState {
    pub(crate) provider_id: String,
    pub(crate) provider_name: String,
    pub(crate) manual_model_placeholder: String,
    pub(crate) default_manual_model: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LocalProviderUnavailableState {
    pub(crate) provider_id: String,
    pub(crate) provider_name: String,
    pub(crate) manual_model_placeholder: String,
    pub(crate) default_manual_model: String,
    pub(crate) message: String,
    pub(crate) can_start_provider: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ProviderModelSelectionState {
    pub(crate) provider_id: String,
    pub(crate) provider_name: String,
    pub(crate) manual_model_placeholder: String,
    models: Vec<ModelPreset>,
    selected_filtered_idx: usize,
    filter_query: String,
    using_unverified_models: bool,
}

impl ProviderModelSelectionState {
    pub(crate) fn new(
        provider_id: String,
        provider_name: String,
        manual_model_placeholder: String,
        models: Vec<ModelPreset>,
    ) -> Option<Self> {
        let picker_ready_models: Vec<ModelPreset> = models
            .iter()
            .filter(|preset| preset.show_in_picker)
            .cloned()
            .collect();
        let using_unverified_models = picker_ready_models.is_empty() && !models.is_empty();
        let models = if using_unverified_models {
            models
        } else {
            picker_ready_models
        };
        if models.is_empty() {
            return None;
        }
        let mut models = models;
        sort_models_for_provider_picker(provider_id.as_str(), &mut models);

        Some(Self {
            provider_id,
            provider_name,
            manual_model_placeholder,
            models,
            selected_filtered_idx: 0,
            filter_query: String::new(),
            using_unverified_models,
        })
    }

    pub(crate) fn using_unverified_models(&self) -> bool {
        self.using_unverified_models
    }

    pub(crate) fn models(&self) -> &[ModelPreset] {
        &self.models
    }

    pub(crate) fn filtered_indices(&self) -> Vec<usize> {
        let query = self.filter_query.trim().to_ascii_lowercase();
        if query.is_empty() {
            return (0..self.models.len()).collect();
        }

        self.models
            .iter()
            .enumerate()
            .filter(|(_, preset)| {
                let haystack = format!(
                    "{} {} {}",
                    preset.model, preset.display_name, preset.description
                );
                haystack.to_ascii_lowercase().contains(query.as_str())
            })
            .map(|(idx, _)| idx)
            .collect()
    }

    pub(crate) fn move_selection(&mut self, delta: isize) {
        let len = self.filtered_indices().len();
        if len == 0 {
            return;
        }

        let len = len as isize;
        self.selected_filtered_idx =
            (self.selected_filtered_idx as isize + delta).rem_euclid(len) as usize;
    }

    pub(crate) fn filter_query(&self) -> &str {
        &self.filter_query
    }

    pub(crate) fn has_filter(&self) -> bool {
        !self.filter_query.is_empty()
    }

    pub(crate) fn push_filter_char(&mut self, c: char) {
        self.filter_query.push(c);
        self.selected_filtered_idx = 0;
    }

    pub(crate) fn pop_filter_char(&mut self) {
        self.filter_query.pop();
        self.clamp_selection();
    }

    pub(crate) fn replace_filter_query(&mut self, query: String) {
        self.filter_query = query;
        self.selected_filtered_idx = 0;
        self.clamp_selection();
    }

    pub(crate) fn clear_filter(&mut self) {
        self.filter_query.clear();
        self.selected_filtered_idx = 0;
    }

    fn clamp_selection(&mut self) {
        let filtered_len = self.filtered_indices().len();
        if filtered_len == 0 {
            self.selected_filtered_idx = 0;
            return;
        }
        self.selected_filtered_idx = self.selected_filtered_idx.min(filtered_len - 1);
    }

    pub(crate) fn selected_idx(&self) -> usize {
        self.selected_filtered_idx
    }

    pub(crate) fn selected_model(&self) -> Option<ModelPreset> {
        let filtered_indices = self.filtered_indices();
        filtered_indices
            .get(self.selected_filtered_idx)
            .and_then(|idx| self.models.get(*idx))
            .cloned()
    }

    pub(crate) fn select_model(&mut self, model_id: &str) {
        let Some(model_idx) = self
            .models
            .iter()
            .position(|preset| preset.model == model_id)
        else {
            return;
        };
        let filtered_indices = self.filtered_indices();
        if let Some(filtered_idx) = filtered_indices.iter().position(|idx| *idx == model_idx) {
            self.selected_filtered_idx = filtered_idx;
        }
    }
}

pub(crate) fn sort_models_for_provider_picker(provider_id: &str, models: &mut [ModelPreset]) {
    if !matches!(
        base_provider_id(provider_id),
        OPENAI_PROVIDER_ID | OPENAI_API_KEY_PROVIDER_ID
    ) {
        return;
    }

    models.sort_by_key(|preset| {
        OPENAI_PICKER_PRIORITY_MODELS
            .iter()
            .position(|model| preset.model == *model)
            .unwrap_or(OPENAI_PICKER_PRIORITY_MODELS.len())
    });
}

fn base_provider_id(provider_id: &str) -> &str {
    provider_id
        .split_once("::")
        .map(|(base_provider_id, _)| base_provider_id)
        .unwrap_or(provider_id)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReasoningChoice {
    pub(crate) stored: Option<ReasoningEffortConfig>,
    pub(crate) label: String,
    pub(crate) description: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct ProviderReasoningSelectionState {
    pub(crate) provider_id: String,
    pub(crate) provider_name: String,
    pub(crate) model: String,
    choices: Vec<ReasoningChoice>,
    selected_idx: usize,
}

impl ProviderReasoningSelectionState {
    pub(crate) fn new(
        provider_id: String,
        provider_name: String,
        preset: ModelPreset,
    ) -> Option<Self> {
        let mut choices: Vec<ReasoningChoice> = ReasoningEffortConfig::iter()
            .filter(|effort| {
                preset
                    .supported_reasoning_efforts
                    .iter()
                    .any(|option| option.effort == *effort)
            })
            .map(|effort| {
                let description = preset
                    .supported_reasoning_efforts
                    .iter()
                    .find(|option| option.effort == effort)
                    .map(|option| option.description.clone())
                    .filter(|description| !description.is_empty());
                ReasoningChoice {
                    stored: Some(effort),
                    label: reasoning_effort_label(effort).to_string(),
                    description,
                }
            })
            .collect();

        if choices.len() <= 1 {
            return None;
        }

        if let Some(default_choice) = choices
            .iter_mut()
            .find(|choice| choice.stored == Some(preset.default_reasoning_effort))
        {
            default_choice.label.push_str(" (default)");
        }

        let selected_idx = choices
            .iter()
            .position(|choice| choice.stored == Some(preset.default_reasoning_effort))
            .unwrap_or(0);

        Some(Self {
            provider_id,
            provider_name,
            model: preset.model,
            choices,
            selected_idx,
        })
    }

    pub(crate) fn move_selection(&mut self, delta: isize) {
        if self.choices.is_empty() {
            return;
        }

        let len = self.choices.len() as isize;
        self.selected_idx = (self.selected_idx as isize + delta).rem_euclid(len) as usize;
    }

    pub(crate) fn choices(&self) -> &[ReasoningChoice] {
        &self.choices
    }

    pub(crate) fn selected_idx(&self) -> usize {
        self.selected_idx
    }

    pub(crate) fn selected_effort(&self) -> Option<ReasoningEffortConfig> {
        self.choices
            .get(self.selected_idx)
            .and_then(|choice| choice.stored)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ManualModelEntryState {
    pub(crate) provider_id: String,
    pub(crate) provider_name: String,
    pub(crate) placeholder: String,
    pub(crate) value: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ManualModelFallbackState {
    pub(crate) provider_id: String,
    pub(crate) provider_name: String,
    pub(crate) manual_model_placeholder: String,
    pub(crate) default_manual_model: String,
    pub(crate) message: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ProviderModelLoadResolution {
    Picker {
        state: ProviderModelSelectionState,
        info_message: Option<String>,
    },
    LocalProviderUnavailable(LocalProviderUnavailableState),
    ManualModelFallback(ManualModelFallbackState),
}

pub(crate) async fn resolve_provider_model_load_resolution(
    loading_state: LoadingProviderModelsState,
    result: Result<Vec<ModelPreset>, String>,
) -> ProviderModelLoadResolution {
    match result {
        Ok(models) => {
            if let Some(mismatch) = detect_provider_model_mismatch(
                loading_state.provider_id.as_str(),
                models.as_slice(),
            ) {
                return ProviderModelLoadResolution::ManualModelFallback(
                    ManualModelFallbackState {
                        provider_id: loading_state.provider_id,
                        provider_name: loading_state.provider_name.clone(),
                        manual_model_placeholder: loading_state.manual_model_placeholder,
                        default_manual_model: loading_state.default_manual_model,
                        message: format!(
                            "Loaded models for {} while {} was selected. To avoid choosing a model for the wrong provider, go back and choose the provider again or enter a model id manually.",
                            mismatch.name, loading_state.provider_name
                        ),
                    },
                );
            }
            if let Some(state) = ProviderModelSelectionState::new(
                loading_state.provider_id.clone(),
                loading_state.provider_name.clone(),
                loading_state.manual_model_placeholder.clone(),
                models,
            ) {
                let info_message = state.using_unverified_models().then(|| {
                    format!(
                        "{} did not advertise enough compatibility metadata. Showing the live provider model list anyway.",
                        loading_state.provider_name
                    )
                });
                ProviderModelLoadResolution::Picker {
                    state,
                    info_message,
                }
            } else if let Some(state) =
                resolve_local_provider_unavailable_state(&loading_state).await
            {
                ProviderModelLoadResolution::LocalProviderUnavailable(state)
            } else {
                let message = no_models_message(
                    loading_state.provider_name.as_str(),
                    loading_state.provider_id.as_str(),
                )
                .unwrap_or_else(|| {
                    format!(
                        "{} did not advertise any picker-ready models. Enter a model name manually.",
                        loading_state.provider_name
                    )
                });
                ProviderModelLoadResolution::ManualModelFallback(ManualModelFallbackState {
                    provider_id: loading_state.provider_id,
                    provider_name: loading_state.provider_name,
                    manual_model_placeholder: loading_state.manual_model_placeholder,
                    default_manual_model: loading_state.default_manual_model,
                    message,
                })
            }
        }
        Err(err) => {
            if let Some(state) = resolve_local_provider_unavailable_state(&loading_state).await {
                ProviderModelLoadResolution::LocalProviderUnavailable(state)
            } else {
                ProviderModelLoadResolution::ManualModelFallback(ManualModelFallbackState {
                    provider_id: loading_state.provider_id,
                    provider_name: loading_state.provider_name.clone(),
                    manual_model_placeholder: loading_state.manual_model_placeholder,
                    default_manual_model: loading_state.default_manual_model,
                    message: format!(
                        "Failed to load models for {}: {err}. Enter a model name manually.",
                        loading_state.provider_name
                    ),
                })
            }
        }
    }
}

fn detect_provider_model_mismatch(
    selected_provider_id: &str,
    models: &[ModelPreset],
) -> Option<&'static BundledProviderCatalogEntry> {
    let selected_base_provider_id = base_provider_id(selected_provider_id);
    if !bundled_provider_catalog()
        .iter()
        .any(|entry| entry.id == selected_base_provider_id)
    {
        return None;
    }

    let returned_model_ids: HashSet<&str> = models
        .iter()
        .map(|preset| preset.model.as_str())
        .filter(|model| !model.trim().is_empty())
        .collect();
    if returned_model_ids.is_empty() {
        return None;
    }

    bundled_provider_catalog().iter().find(|entry| {
        entry.id != selected_base_provider_id && {
            let catalog_model_ids: HashSet<&str> =
                entry.models.iter().map(|model| model.id.as_str()).collect();
            !catalog_model_ids.is_empty()
                && returned_model_ids
                    .iter()
                    .all(|model| catalog_model_ids.contains(model))
        }
    })
}

async fn resolve_local_provider_unavailable_state(
    loading_state: &LoadingProviderModelsState,
) -> Option<LocalProviderUnavailableState> {
    if !is_local_provider(loading_state.provider_id.as_str()) {
        return None;
    }
    if is_local_provider_running(loading_state.provider_id.as_str())
        .await
        .unwrap_or(false)
    {
        return None;
    }
    let message = not_running_message(
        loading_state.provider_name.as_str(),
        loading_state.provider_id.as_str(),
    )?;
    Some(LocalProviderUnavailableState {
        provider_id: loading_state.provider_id.clone(),
        provider_name: loading_state.provider_name.clone(),
        manual_model_placeholder: loading_state.manual_model_placeholder.clone(),
        default_manual_model: loading_state.default_manual_model.clone(),
        message,
        can_start_provider: can_start_local_provider(loading_state.provider_id.as_str()),
    })
}

impl ManualModelEntryState {
    pub(crate) fn new(
        provider_id: String,
        provider_name: String,
        placeholder: String,
        default_value: String,
    ) -> Self {
        Self {
            provider_id,
            provider_name,
            placeholder,
            value: default_value,
        }
    }

    pub(crate) fn push_char(&mut self, c: char) {
        self.value.push(c);
    }

    pub(crate) fn pop_char(&mut self) {
        self.value.pop();
    }

    pub(crate) fn replace_value(&mut self, value: String) {
        self.value = value;
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.value.trim().is_empty() {
            return Err("Model cannot be empty".to_string());
        }
        Ok(())
    }

    pub(crate) fn selected_model(&self) -> String {
        self.value.trim().to_string()
    }
}

fn reasoning_effort_label(effort: ReasoningEffortConfig) -> &'static str {
    match effort {
        ReasoningEffortConfig::None => "No reasoning",
        ReasoningEffortConfig::Minimal => "Minimal",
        ReasoningEffortConfig::Low => "Low",
        ReasoningEffortConfig::Medium => "Medium",
        ReasoningEffortConfig::High => "High",
        ReasoningEffortConfig::XHigh => "Extra high",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_protocol::openai_models::ReasoningControl;
    use codex_protocol::openai_models::default_input_modalities;

    #[test]
    fn catalog_provider_offers_all_chat_harnesses_regardless_of_passed_wire() {
        // Groq is a `chat` provider in the bundled catalog. Every entry point
        // should offer the full chat harness set, even when the caller doesn't
        // know the wire (None) or passes a stale/incorrect one (Responses).
        for passed_wire in [None, Some(WireApi::Responses), Some(WireApi::Chat)] {
            // Found by id.
            let by_id = harness_choices_for_provider_model(
                "groq",
                Some("Groq"),
                Some("https://api.groq.com/openai/v1"),
                passed_wire,
                Some("openai/gpt-oss-120b"),
            );
            // Found only by base URL (unknown id).
            let by_base_url = harness_choices_for_provider_model(
                "custom-groq",
                Some("Groq"),
                Some("https://api.groq.com/openai/v1"),
                passed_wire,
                Some("openai/gpt-oss-120b"),
            );
            assert!(
                by_id.len() > 1,
                "by_id wire={passed_wire:?} len={}",
                by_id.len()
            );
            assert!(
                by_base_url.len() > 1,
                "by_base_url wire={passed_wire:?} len={}",
                by_base_url.len()
            );
        }
    }

    fn loading_state(provider_id: &str, provider_name: &str) -> LoadingProviderModelsState {
        LoadingProviderModelsState {
            provider_id: provider_id.to_string(),
            provider_name: provider_name.to_string(),
            manual_model_placeholder: "model-id".to_string(),
            default_manual_model: String::new(),
        }
    }

    fn model_preset(model: &str) -> ModelPreset {
        ModelPreset {
            id: model.to_string(),
            model: model.to_string(),
            display_name: model.to_string(),
            description: String::new(),
            default_reasoning_effort: ReasoningEffortConfig::None,
            supported_reasoning_efforts: Vec::new(),
            reasoning_control: ReasoningControl::None,
            supports_thinking_toggle: false,
            supports_personality: false,
            is_default: false,
            upgrade: None,
            show_in_picker: true,
            availability_nux: None,
            supported_in_api: true,
            input_modalities: default_input_modalities(),
            additional_speed_tiers: Vec::new(),
        }
    }

    #[tokio::test]
    async fn openai_api_key_alias_does_not_reject_known_model_lists() {
        let resolution = resolve_provider_model_load_resolution(
            loading_state("openai_api_key", "OpenAI (API key)"),
            Ok(vec![model_preset("k2p5"), model_preset("kimi-k2-thinking")]),
        )
        .await;

        assert!(matches!(
            resolution,
            ProviderModelLoadResolution::Picker { .. }
        ));
    }

    #[tokio::test]
    async fn kimi_code_accepts_kimi_code_model_list() {
        let resolution = resolve_provider_model_load_resolution(
            loading_state("kimi-for-coding", "Kimi Code"),
            Ok(vec![model_preset("k2p5"), model_preset("kimi-k2-thinking")]),
        )
        .await;

        assert!(matches!(
            resolution,
            ProviderModelLoadResolution::Picker { .. }
        ));
    }
}
