use super::*;
use crate::bottom_pane::custom_prompt_view::CustomPromptView;
use crate::config_write_edits::preferred_harness_for_provider;
use crate::onboarding::local_provider::start_hint as local_provider_start_hint;
use crate::onboarding::model_selection::LoadingProviderModelsState;
use crate::onboarding::model_selection::LocalProviderUnavailableState;
use crate::onboarding::model_selection::ProviderModelLoadResolution;
use crate::onboarding::model_selection::ProviderModelSelectionState;
use crate::onboarding::provider_setup::provider_preset_by_id;
use crate::provider_model_flow::ProviderChoiceAction;
use crate::provider_model_flow::model_picker_provider_choices;
#[cfg(test)]
use crate::provider_model_flow::model_picker_provider_choices_with_snapshot;
#[cfg(test)]
use crate::provider_readiness::ProviderReadinessSnapshot;

impl ChatWidget {
    fn loading_provider_models_state(
        &self,
        provider_id: &str,
        provider_name: &str,
    ) -> LoadingProviderModelsState {
        let preset = provider_preset_by_id(provider_id);
        LoadingProviderModelsState {
            provider_id: provider_id.to_string(),
            provider_name: provider_name.to_string(),
            manual_model_placeholder: preset
                .as_ref()
                .map(|preset| preset.model_placeholder.clone())
                .unwrap_or_else(|| "model-name".to_string()),
            default_manual_model: preset
                .and_then(|preset| preset.default_model)
                .unwrap_or_default(),
        }
    }

    fn provider_harness_label(&self, provider_id: &str, provider_name: &str) -> Option<String> {
        let provider = self.config.model_providers.get(provider_id);
        preferred_harness_for_provider(
            provider_id,
            Some(provider_name),
            provider.and_then(|entry| entry.base_url.as_deref()),
            provider.map(|entry| entry.wire_api),
        )
        .map(ToOwned::to_owned)
    }

    fn provider_model_prompt_context(
        &self,
        provider_id: &str,
        provider_name: &str,
        starts_new_chat: bool,
    ) -> String {
        let mut context = if starts_new_chat {
            "This will update config and start a new chat.".to_string()
        } else {
            "This updates the current provider without starting a new chat.".to_string()
        };
        if let Some(harness) = self.provider_harness_label(provider_id, provider_name) {
            context.push_str(&format!(" Harness: {harness}."));
        }
        context
    }

    pub(crate) fn supported_reasoning_choice_count(preset: &ModelPreset) -> usize {
        let count = ReasoningEffortConfig::iter()
            .filter(|effort| {
                preset
                    .supported_reasoning_efforts
                    .iter()
                    .any(|option| option.effort == *effort)
            })
            .count();
        if count > 0 {
            count
        } else if Self::supports_thinking_toggle_only(preset) {
            2
        } else {
            1
        }
    }

    pub(crate) fn supports_thinking_toggle_only(preset: &ModelPreset) -> bool {
        preset.supported_reasoning_efforts.is_empty()
            && preset.default_reasoning_effort != ReasoningEffortConfig::None
    }

    pub(crate) fn set_model_catalog(&mut self, model_catalog: Arc<ModelCatalog>) {
        self.model_catalog = model_catalog;
    }

    pub(crate) fn open_model_provider_popup(&mut self) {
        self.show_model_provider_popup(/*startup_mode*/ false);
    }

    pub(crate) fn open_startup_provider_popup(&mut self) {
        self.show_model_provider_popup(/*startup_mode*/ true);
    }

    fn show_model_provider_popup(&mut self, startup_mode: bool) {
        self.show_model_provider_popup_with_choices(
            startup_mode,
            model_picker_provider_choices(&self.config),
        );
    }

    fn show_model_provider_popup_with_choices(
        &mut self,
        startup_mode: bool,
        providers: Vec<crate::provider_model_flow::ProviderChoice>,
    ) {
        let items: Vec<SelectionItem> = providers
            .into_iter()
            .map(|provider| {
                let provider_id = provider.id.clone();
                let provider_name = provider.name.clone();
                let loading_state = self
                    .loading_provider_models_state(provider_id.as_str(), provider_name.as_str());
                let provider_name_search = provider.name.clone();
                let provider_description_search = provider.description.clone();
                let actions: Vec<SelectionAction> = match provider.action {
                    ProviderChoiceAction::Existing => vec![Box::new(move |tx| {
                        tx.send(AppEvent::LoadProviderModels {
                            loading_state: loading_state.clone(),
                        });
                    })],
                    ProviderChoiceAction::QuickAdd(preset) => vec![Box::new(move |tx| {
                        tx.send(AppEvent::ConfigureProviderPresetAndLoadModels {
                            preset: preset.clone(),
                        });
                    })],
                };
                SelectionItem {
                    name: provider.name,
                    description: Some(provider.description),
                    is_current: !startup_mode && provider.is_current,
                    actions,
                    dismiss_on_select: true,
                    search_value: Some(format!(
                        "{} {} {}",
                        provider.id, provider_name_search, provider_description_search
                    )),
                    ..Default::default()
                }
            })
            .collect();

        let (title, subtitle) = if startup_mode {
            (
                Some("Welcome to Open Interpreter.".to_string()),
                Some("Choose a provider to get started.".to_string()),
            )
        } else {
            (
                Some("Select Provider".to_string()),
                Some(
                    "Choose a provider for the next chat. Unconfigured providers will be added to config."
                        .to_string(),
                ),
            )
        };

        self.bottom_pane.show_selection_view(SelectionViewParams {
            title,
            subtitle,
            footer_hint: Some("Type to filter • Enter to continue • Esc to dismiss".into()),
            items,
            is_searchable: true,
            search_placeholder: Some("Filter providers".to_string()),
            ..Default::default()
        });
    }

    #[cfg(test)]
    pub(crate) fn open_model_provider_popup_with_snapshot(
        &mut self,
        startup_mode: bool,
        snapshot: &ProviderReadinessSnapshot,
    ) {
        self.show_model_provider_popup_with_choices(
            startup_mode,
            model_picker_provider_choices_with_snapshot(&self.config, snapshot),
        );
    }

    pub(crate) fn open_custom_provider_name_prompt(
        &mut self,
        preset: crate::onboarding::provider_setup::ProviderPreset,
    ) {
        let tx = self.app_event_tx.clone();
        let preset_for_submit = preset;
        let view = CustomPromptView::new(
            "Provider name".to_string(),
            "Choose the name that should appear in /model and config".to_string(),
            String::new(),
            Some("This provider will be saved to config and start a new chat.".to_string()),
            Box::new(move |provider_name: String| {
                let provider_name = provider_name.trim().to_string();
                if provider_name.is_empty() {
                    return;
                }
                tx.send(AppEvent::OpenCustomProviderBaseUrlPrompt {
                    preset: preset_for_submit.clone(),
                    provider_name,
                });
            }),
        );
        self.bottom_pane.show_view(Box::new(view));
    }

    pub(crate) fn open_custom_provider_base_url_prompt(
        &mut self,
        preset: crate::onboarding::provider_setup::ProviderPreset,
        provider_name: String,
    ) {
        let tx = self.app_event_tx.clone();
        let preset_for_submit = preset.clone();
        let view = CustomPromptView::new(
            format!("{provider_name} base URL"),
            "https://api.example.com/v1".to_string(),
            preset.base_url,
            Some(format!(
                "Provider: {provider_name}. This will be saved to config and start a new chat."
            )),
            Box::new(move |base_url: String| {
                tx.send(AppEvent::OpenCustomProviderApiKeyPrompt {
                    preset: preset_for_submit.clone(),
                    provider_name: provider_name.clone(),
                    base_url,
                });
            }),
        );
        self.bottom_pane.show_view(Box::new(view));
    }

    pub(crate) fn open_custom_provider_api_key_prompt(
        &mut self,
        preset: crate::onboarding::provider_setup::ProviderPreset,
        provider_name: String,
        base_url: String,
    ) {
        let tx = self.app_event_tx.clone();
        let env_prefill = preset
            .api_key_env_var_name(Some(provider_name.as_str()))
            .and_then(|env_var_name| {
                crate::login_support::read_env_var_trimmed(env_var_name.as_str())
                    .map(|value| (env_var_name, value))
            });
        let initial_text = env_prefill
            .as_ref()
            .map(|(_, value)| value.clone())
            .unwrap_or_default();
        let context_label = Some(match env_prefill.as_ref() {
            Some((env_var_name, _)) => format!(
                "Provider: {provider_name} • Base URL: {} • Detected {env_var_name}",
                base_url.trim()
            ),
            None => format!("Provider: {provider_name} • Base URL: {}", base_url.trim()),
        });
        let preset_for_submit = preset;
        let view = CustomPromptView::new(
            format!("{provider_name} API key"),
            "Leave blank if this endpoint does not require an API key".to_string(),
            initial_text,
            context_label,
            Box::new(move |api_key: String| {
                let api_key_prefilled_from_env = env_prefill
                    .as_ref()
                    .is_some_and(|(_, value)| value == api_key.trim());
                tx.send(AppEvent::ConfigureCustomProviderAndLoadModels {
                    preset: preset_for_submit.clone(),
                    provider_name: provider_name.clone(),
                    base_url: base_url.clone(),
                    api_key,
                    api_key_prefilled_from_env,
                });
            }),
        )
        .allow_empty_submit();
        self.bottom_pane.show_view(Box::new(view));
    }

    pub(crate) fn open_custom_model_prompt(&mut self) {
        let tx = self.app_event_tx.clone();
        let current_model = self.current_model().to_string();
        let provider_id = self.config.model_provider_id.clone();
        let provider_name = self
            .config
            .model_providers
            .get(provider_id.as_str())
            .map(|provider| provider.name.clone())
            .unwrap_or_else(|| provider_id.clone());
        let view = CustomPromptView::new(
            "Custom model name".to_string(),
            "Type any model id, then press Enter".to_string(),
            current_model,
            Some(self.provider_model_prompt_context(
                provider_id.as_str(),
                provider_name.as_str(),
                /*starts_new_chat*/ false,
            )),
            Box::new(move |model: String| {
                let model = model.trim().to_string();
                if model.is_empty() {
                    return;
                }
                tx.send(AppEvent::PersistModelSelection {
                    model,
                    effort: None,
                });
            }),
        );
        self.bottom_pane.show_view(Box::new(view));
        self.add_info_message(
            format!(
                "Enter a model slug for {provider_name}. Reasoning can be adjusted afterward if the model supports it."
            ),
            /*hint*/ None,
        );
    }

    pub(crate) fn open_custom_model_prompt_for_provider(
        &mut self,
        provider_id: String,
        provider_name: String,
    ) {
        self.open_custom_model_prompt_for_provider_with_initial_value(
            provider_id,
            provider_name,
            None,
        );
    }

    pub(crate) fn open_custom_model_prompt_for_provider_with_initial_value(
        &mut self,
        provider_id: String,
        provider_name: String,
        initial_text: Option<String>,
    ) {
        let tx = self.app_event_tx.clone();
        let view = CustomPromptView::new(
            format!("{provider_name} model name"),
            "Type any model id, then press Enter".to_string(),
            initial_text.unwrap_or_default(),
            Some(self.provider_model_prompt_context(
                provider_id.as_str(),
                provider_name.as_str(),
                /*starts_new_chat*/ true,
            )),
            Box::new(move |model: String| {
                let model = model.trim().to_string();
                if model.is_empty() {
                    return;
                }
                tx.send(AppEvent::PersistProviderModelSelection {
                    provider_id: provider_id.clone(),
                    provider_name: provider_name.clone(),
                    model,
                    effort: None,
                });
            }),
        );
        self.bottom_pane.show_view(Box::new(view));
    }

    pub(crate) fn present_provider_model_load_resolution(
        &mut self,
        resolution: ProviderModelLoadResolution,
    ) {
        match resolution {
            ProviderModelLoadResolution::Picker {
                state,
                info_message,
            } => {
                if let Some(message) = info_message {
                    self.add_info_message(message, /*hint*/ None);
                }
                self.open_model_popup_for_provider_state(state);
            }
            ProviderModelLoadResolution::LocalProviderUnavailable(state) => {
                self.open_local_provider_unavailable_popup(state);
            }
            ProviderModelLoadResolution::ManualModelFallback(state) => {
                self.add_info_message(state.message, /*hint*/ None);
                self.open_custom_model_prompt_for_provider_with_initial_value(
                    state.provider_id,
                    state.provider_name,
                    Some(state.default_manual_model),
                );
            }
        }
    }

    pub(crate) fn open_model_popup_for_provider(
        &mut self,
        provider_id: String,
        provider_name: String,
        presets: Vec<ModelPreset>,
    ) {
        let Some(state) = ProviderModelSelectionState::new(
            provider_id.clone(),
            provider_name.clone(),
            format!("{provider_name} model name"),
            presets,
        ) else {
            self.open_custom_model_prompt_for_provider(provider_id, provider_name);
            return;
        };
        self.open_model_popup_for_provider_state(state);
    }

    fn open_model_popup_for_provider_state(&mut self, state: ProviderModelSelectionState) {
        let provider_id = state.provider_id.clone();
        let provider_name = state.provider_name.clone();
        let filtered = state.models().to_vec();

        if provider_id == self.config.model_provider_id {
            self.open_all_models_popup(filtered);
            return;
        }

        let items: Vec<SelectionItem> = filtered
            .into_iter()
            .map(|preset| {
                let description =
                    (!preset.description.is_empty()).then_some(preset.description.clone());
                let preset_for_action = preset.clone();
                let provider_id_for_action = provider_id.clone();
                let provider_name_for_action = provider_name.clone();
                let single_supported_effort = Self::supported_reasoning_choice_count(&preset) == 1;
                let actions: Vec<SelectionAction> = vec![Box::new(move |tx| {
                    tx.send(AppEvent::OpenReasoningPopupForProvider {
                        provider_id: provider_id_for_action.clone(),
                        provider_name: provider_name_for_action.clone(),
                        model: preset_for_action.clone(),
                    });
                })];
                SelectionItem {
                    name: preset.model.clone(),
                    description,
                    is_default: preset.is_default,
                    actions,
                    dismiss_on_select: single_supported_effort,
                    search_value: Some(format!(
                        "{} {} {}",
                        preset.model, preset.display_name, preset.description
                    )),
                    ..Default::default()
                }
            })
            .chain(std::iter::once({
                let provider_id_for_action = provider_id.clone();
                let provider_name_for_action = provider_name.clone();
                SelectionItem {
                    name: "Custom model name".to_string(),
                    description: Some(
                        "Type a model id that this provider accepts, even if it is not listed here."
                            .to_string(),
                    ),
                    actions: vec![Box::new(move |tx| {
                        tx.send(AppEvent::OpenCustomProviderModelPrompt {
                            provider_id: provider_id_for_action.clone(),
                            provider_name: provider_name_for_action.clone(),
                            initial_text: None,
                        });
                    })],
                    dismiss_on_select: true,
                    search_value: Some("custom manual typed model".to_string()),
                    ..Default::default()
                }
            }))
            .collect();

        let title = format!("Select Model for {provider_name}");
        let subtitle =
            match self.provider_harness_label(provider_id.as_str(), provider_name.as_str()) {
                Some(harness) => format!(
                    "This will start a new chat with the selected provider. Harness: {harness}."
                ),
                None => "This will start a new chat with the selected provider.".to_string(),
            };
        let header = self.model_menu_header(title.as_str(), subtitle.as_str());
        self.bottom_pane.show_selection_view(SelectionViewParams {
            footer_hint: Some(
                "Type to filter • Enter to configure reasoning • Esc to dismiss".into(),
            ),
            items,
            header,
            is_searchable: true,
            search_placeholder: Some("Filter models".to_string()),
            ..Default::default()
        });
    }

    fn open_local_provider_unavailable_popup(&mut self, state: LocalProviderUnavailableState) {
        let mut items: Vec<SelectionItem> = Vec::new();
        if state.can_start_provider {
            let loading_state = LoadingProviderModelsState {
                provider_id: state.provider_id.clone(),
                provider_name: state.provider_name.clone(),
                manual_model_placeholder: state.manual_model_placeholder.clone(),
                default_manual_model: state.default_manual_model.clone(),
            };
            items.push(SelectionItem {
                name: format!("Start {}", state.provider_name),
                description: local_provider_start_hint(state.provider_id.as_str())
                    .map(ToOwned::to_owned),
                actions: vec![Box::new(move |tx| {
                    tx.send(AppEvent::StartLocalProviderAndLoadModels {
                        loading_state: loading_state.clone(),
                    });
                })],
                dismiss_on_select: true,
                ..Default::default()
            });
        }

        let provider_id = state.provider_id.clone();
        let provider_name = state.provider_name.clone();
        let default_model = state.default_manual_model.clone();
        items.push(SelectionItem {
            name: "Type model name manually".to_string(),
            description: Some(
                "Use this if the local server is already running or you know the model id."
                    .to_string(),
            ),
            actions: vec![Box::new(move |tx| {
                tx.send(AppEvent::OpenCustomProviderModelPrompt {
                    provider_id: provider_id.clone(),
                    provider_name: provider_name.clone(),
                    initial_text: Some(default_model.clone()),
                });
            })],
            dismiss_on_select: true,
            ..Default::default()
        });

        let title = format!("{} is unavailable", state.provider_name);
        let header = self.model_menu_header(title.as_str(), state.message.as_str());
        self.bottom_pane.show_selection_view(SelectionViewParams {
            footer_hint: Some("Enter to continue • Esc to dismiss".into()),
            items,
            header,
            ..Default::default()
        });
    }

    pub(crate) fn open_reasoning_popup_for_provider(
        &mut self,
        provider_id: String,
        provider_name: String,
        preset: ModelPreset,
    ) {
        let default_effort: ReasoningEffortConfig = preset.default_reasoning_effort;
        let supported = preset.supported_reasoning_efforts;

        if supported.is_empty() {
            if default_effort == ReasoningEffortConfig::None {
                self.app_event_tx
                    .send(AppEvent::PersistProviderModelSelection {
                        provider_id,
                        provider_name,
                        model: preset.model,
                        effort: None,
                    });
                return;
            }

            let model = preset.model.clone();
            let on_actions: Vec<SelectionAction> = vec![Box::new({
                let provider_id = provider_id.clone();
                let provider_name = provider_name.clone();
                let model = model.clone();
                move |tx| {
                    tx.send(AppEvent::PersistProviderModelSelection {
                        provider_id: provider_id.clone(),
                        provider_name: provider_name.clone(),
                        model: model.clone(),
                        effort: None,
                    });
                }
            })];
            let off_actions: Vec<SelectionAction> = vec![Box::new({
                let provider_name = provider_name.clone();
                move |tx| {
                    tx.send(AppEvent::PersistProviderModelSelection {
                        provider_id: provider_id.clone(),
                        provider_name: provider_name.clone(),
                        model: model.clone(),
                        effort: Some(ReasoningEffortConfig::None),
                    });
                }
            })];

            let mut header = ColumnRenderable::new();
            header.push(Line::from(
                format!("Select Thinking Mode for {}", preset.model).bold(),
            ));
            header.push(Line::from(
                format!("{provider_name} will start a new chat with this selection.").dim(),
            ));

            self.bottom_pane.show_selection_view(SelectionViewParams {
                header: Box::new(header),
                footer_hint: Some(standard_popup_hint_line()),
                items: vec![
                    SelectionItem {
                        name: "On (default)".to_string(),
                        description: Some(
                            "Use the model's default thinking behavior without forcing an effort level."
                                .to_string(),
                        ),
                        actions: on_actions,
                        dismiss_on_select: true,
                        ..Default::default()
                    },
                    SelectionItem {
                        name: "Off".to_string(),
                        description: Some(
                            "Disable thinking explicitly for this model.".to_string(),
                        ),
                        actions: off_actions,
                        dismiss_on_select: true,
                        ..Default::default()
                    },
                ],
                ..Default::default()
            });
            return;
        }

        struct EffortChoice {
            stored: Option<ReasoningEffortConfig>,
            display: ReasoningEffortConfig,
        }

        let mut choices: Vec<EffortChoice> = ReasoningEffortConfig::iter()
            .filter(|effort| supported.iter().any(|option| option.effort == *effort))
            .map(|effort| EffortChoice {
                stored: Some(effort),
                display: effort,
            })
            .collect();

        if choices.is_empty() {
            choices.push(EffortChoice {
                stored: Some(default_effort),
                display: default_effort,
            });
        }

        if choices.len() == 1 {
            let selected_effort = choices.first().and_then(|choice| choice.stored);
            self.app_event_tx
                .send(AppEvent::PersistProviderModelSelection {
                    provider_id,
                    provider_name,
                    model: preset.model,
                    effort: selected_effort,
                });
            return;
        }

        let default_choice: Option<ReasoningEffortConfig> = choices
            .iter()
            .any(|choice| choice.stored == Some(default_effort))
            .then_some(Some(default_effort))
            .flatten()
            .or_else(|| choices.iter().find_map(|choice| choice.stored))
            .or(Some(default_effort));
        let initial_selected_idx = choices
            .iter()
            .position(|choice| choice.stored == default_choice)
            .or(Some(0));

        let mut items: Vec<SelectionItem> = Vec::new();
        for choice in choices {
            let effort = choice.display;
            let mut effort_label = Self::reasoning_effort_label(effort).to_string();
            if choice.stored == default_choice {
                effort_label.push_str(" (default)");
            }
            let description = choice
                .stored
                .and_then(|stored_effort| {
                    supported
                        .iter()
                        .find(|option| option.effort == stored_effort)
                        .map(|option| option.description.to_string())
                })
                .filter(|text| !text.is_empty());
            let model = preset.model.clone();
            let provider_id_for_action = provider_id.clone();
            let provider_name_for_action = provider_name.clone();
            let actions: Vec<SelectionAction> = vec![Box::new(move |tx| {
                tx.send(AppEvent::PersistProviderModelSelection {
                    provider_id: provider_id_for_action.clone(),
                    provider_name: provider_name_for_action.clone(),
                    model: model.clone(),
                    effort: choice.stored,
                });
            })];
            items.push(SelectionItem {
                name: effort_label,
                description,
                actions,
                dismiss_on_select: true,
                ..Default::default()
            });
        }

        let mut header = ColumnRenderable::new();
        header.push(Line::from(
            format!("Select Reasoning Level for {}", preset.model).bold(),
        ));
        header.push(Line::from(
            format!("{provider_name} will start a new chat with this selection.").dim(),
        ));

        self.bottom_pane.show_selection_view(SelectionViewParams {
            header: Box::new(header),
            footer_hint: Some(standard_popup_hint_line()),
            items,
            initial_selected_idx,
            ..Default::default()
        });
    }
}
