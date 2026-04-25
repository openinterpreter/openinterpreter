use codex_model_provider_info::OPENAI_PROVIDER_ID;
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
