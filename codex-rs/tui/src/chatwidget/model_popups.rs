//! Model, collaboration, and reasoning popups for `ChatWidget`.
//!
//! These surfaces are tightly related because changing one often redirects
//! into another, especially while Plan mode is active.

use super::*;
use crate::model_catalog::LUNA_RESERVE_MODEL;
use codex_app_server_protocol::InterpreterHarness;
use codex_app_server_protocol::InterpreterProvider;
use codex_login::KIMI_CODE_PROVIDER_ID;

const ULTRA_REASONING_CONCURRENCY_WARNING_THRESHOLD: usize = 8;
const MODEL_PROVIDER_SELECTION_VIEW_ID: &str = "model-provider-selection";
const HARNESS_SELECTION_VIEW_ID: &str = "harness-selection";
pub(super) const MODEL_SELECTION_VIEW_ID: &str = "model-selection";
pub(super) const ALL_MODELS_SELECTION_VIEW_ID: &str = "all-models-selection";

impl ChatWidget {
    /// Open a popup to choose a quick auto model. Selecting "All models"
    /// opens the full picker with every available preset.
    pub(crate) fn open_model_popup(&mut self) {
        if !self.is_session_configured() {
            self.add_info_message(
                "Model selection is disabled until startup completes.".to_string(),
                /*hint*/ None,
            );
            return;
        }

        self.bottom_pane.show_selection_view(SelectionViewParams {
            view_id: Some(MODEL_PROVIDER_SELECTION_VIEW_ID),
            title: Some("Select Provider".to_string()),
            subtitle: Some("Loading available providers...".to_string()),
            items: vec![SelectionItem {
                name: "Loading providers...".to_string(),
                is_disabled: true,
                ..Default::default()
            }],
            ..Default::default()
        });
        self.app_event_tx.send(AppEvent::LoadProviderCatalog);
    }

    pub(crate) fn on_model_provider_catalog_error(&mut self, err: String) {
        if self
            .bottom_pane
            .dismiss_active_view_if_id(MODEL_PROVIDER_SELECTION_VIEW_ID)
        {
            self.add_error_message(format!("Failed to load model providers: {err}"));
        }
    }

    pub(crate) fn open_current_harness_popup(&mut self) {
        if !self.is_session_configured() {
            self.add_info_message(
                "Harness selection is disabled until startup completes.".to_string(),
                /*hint*/ None,
            );
            return;
        }

        let provider_id = self.config.model_provider_id.clone();
        let provider_name = self
            .config
            .model_providers
            .get(provider_id.as_str())
            .map(|provider| provider.name.clone())
            .unwrap_or_else(|| provider_id.clone());
        self.app_event_tx.send(AppEvent::LoadHarnesses {
            provider_id,
            provider_name,
            model: self.current_model().to_string(),
            effort: self.effective_reasoning_effort(),
        });
    }

    pub(crate) fn open_model_provider_popup(&mut self, providers: Vec<InterpreterProvider>) {
        let items = providers
            .into_iter()
            .map(|provider| {
                let provider_id = provider.id;
                let provider_name = provider.name;
                let action_provider_id = provider_id.clone();
                let action_provider_name = provider_name.clone();
                SelectionItem {
                    name: provider_name.clone(),
                    description: Some(provider.description),
                    is_current: provider.is_current,
                    is_default: provider.is_default,
                    actions: vec![Box::new(move |tx| {
                        if action_provider_id == KIMI_CODE_PROVIDER_ID {
                            tx.send(AppEvent::StartKimiCodeLogin {
                                provider_id: action_provider_id.clone(),
                                provider_name: action_provider_name.clone(),
                            });
                        } else {
                            tx.send(AppEvent::LoadProviderModels {
                                provider_id: action_provider_id.clone(),
                                provider_name: action_provider_name.clone(),
                            });
                        }
                    })],
                    dismiss_on_select: false,
                    dismiss_parent_on_child_accept: true,
                    search_value: Some(format!("{provider_id} {provider_name}")),
                    ..Default::default()
                }
            })
            .collect();

        let _ = self.bottom_pane.replace_selection_view_if_active(
            MODEL_PROVIDER_SELECTION_VIEW_ID,
            SelectionViewParams {
                view_id: Some(MODEL_PROVIDER_SELECTION_VIEW_ID),
                title: Some("Select Provider".to_string()),
                subtitle: Some("Choose a provider for the next chat.".to_string()),
                footer_hint: Some("Type to filter • Enter to continue • Esc to dismiss".into()),
                items,
                is_searchable: true,
                search_placeholder: Some("Filter providers".to_string()),
                ..Default::default()
            },
        );
    }

    pub(crate) fn open_provider_models_loading_popup(&mut self, provider_name: &str) {
        self.bottom_pane.show_selection_view(SelectionViewParams {
            view_id: Some(MODEL_SELECTION_VIEW_ID),
            title: Some(format!("Select Model for {provider_name}")),
            subtitle: Some("Loading available models...".to_string()),
            items: vec![SelectionItem {
                name: "Loading models...".to_string(),
                is_disabled: true,
                ..Default::default()
            }],
            ..Default::default()
        });
    }

    pub(crate) fn on_provider_models_error(
        &mut self,
        provider_id: String,
        provider_name: String,
        err: String,
    ) {
        if self
            .bottom_pane
            .dismiss_active_view_if_id(MODEL_SELECTION_VIEW_ID)
        {
            self.add_error_message(format!("Failed to load models for {provider_name}: {err}"));
            self.open_custom_model_prompt_for_provider(
                provider_id,
                provider_name,
                /*initial_text*/ None,
            );
        }
    }

    pub(super) fn model_menu_header(&self, title: &str, subtitle: &str) -> Box<dyn Renderable> {
        let title = title.to_string();
        let subtitle = subtitle.to_string();
        let mut header = vec![Line::from(title.bold())];
        if !subtitle.is_empty() {
            header.push(Line::from(subtitle.dim()));
        }
        if let Some(warning) = self.model_menu_warning_line() {
            header.push(warning);
        }
        Box::new(Paragraph::new(header).wrap(Wrap { trim: false }))
    }

    fn model_menu_warning_line(&self) -> Option<Line<'static>> {
        let base_url = self.custom_openai_base_url()?;
        let warning = format!(
            "Warning: OpenAI base URL is overridden to {base_url}. Selecting models may not be supported or work properly."
        );
        Some(Line::from(warning.red()))
    }

    fn custom_openai_base_url(&self) -> Option<String> {
        if !self.config.model_provider.is_openai() {
            return None;
        }

        let base_url = self.config.model_provider.base_url.as_ref()?;
        let trimmed = base_url.trim();
        if trimmed.is_empty() {
            return None;
        }

        let normalized = trimmed.trim_end_matches('/');
        if normalized == DEFAULT_OPENAI_BASE_URL {
            return None;
        }

        Some(trimmed.to_string())
    }

    #[allow(dead_code)]
    pub(crate) fn open_model_popup_with_presets(&mut self, presets: Vec<ModelPreset>) {
        if self.restrict_model_picker_to_luna_reserve() {
            self.open_luna_reserve_model_popup(presets, MODEL_SELECTION_VIEW_ID);
            return;
        }
        let presets: Vec<ModelPreset> = presets
            .into_iter()
            .filter(|preset| preset.show_in_picker)
            .collect();

        let current_model = self.current_model();
        let current_label = presets
            .iter()
            .find(|preset| preset.model.as_str() == current_model)
            .map(|preset| preset.display_name.clone())
            .unwrap_or_else(|| self.model_display_name().to_string());

        let (mut auto_presets, other_presets): (Vec<ModelPreset>, Vec<ModelPreset>) = presets
            .into_iter()
            .partition(|preset| Self::is_auto_model(&preset.model));

        if auto_presets.is_empty() {
            self.open_all_models_popup_with_view_id(other_presets, MODEL_SELECTION_VIEW_ID);
            return;
        }

        auto_presets.sort_by_key(|preset| Self::auto_model_order(&preset.model));
        let mut model_ids: Vec<String> = auto_presets
            .iter()
            .map(|preset| preset.model.clone())
            .collect();
        let mut items: Vec<SelectionItem> = auto_presets
            .into_iter()
            .map(|preset| {
                let description =
                    (!preset.description.is_empty()).then_some(preset.description.clone());
                let model = preset.model.clone();
                let requires_advanced_selection =
                    Self::is_advanced_reasoning_effort(&preset.default_reasoning_effort)
                        || preset
                            .supported_reasoning_efforts
                            .iter()
                            .any(|option| Self::is_advanced_reasoning_effort(&option.effort));
                let actions: Vec<SelectionAction> = if requires_advanced_selection {
                    let preset_for_action = preset.clone();
                    vec![Box::new(move |tx| {
                        tx.send(AppEvent::OpenReasoningPopup {
                            model: preset_for_action.clone(),
                        });
                    })]
                } else {
                    let should_prompt_plan_mode_scope = self
                        .should_prompt_plan_mode_reasoning_scope(
                            model.as_str(),
                            Some(preset.default_reasoning_effort.clone()),
                        );
                    self.model_selection_actions(
                        model.clone(),
                        Some(preset.default_reasoning_effort.clone()),
                        should_prompt_plan_mode_scope,
                    )
                };
                SelectionItem {
                    name: preset.display_name.clone(),
                    description,
                    is_current: model.as_str() == current_model,
                    is_default: preset.is_default,
                    secondary_action: if requires_advanced_selection {
                        None
                    } else {
                        self.session_model_selection_action(
                            model.clone(),
                            Some(preset.default_reasoning_effort),
                        )
                    },
                    actions,
                    dismiss_on_select: !requires_advanced_selection,
                    dismiss_parent_on_child_accept: requires_advanced_selection,
                    ..Default::default()
                }
            })
            .collect();

        if !other_presets.is_empty() {
            model_ids.push("All models".to_string());
            let actions: Vec<SelectionAction> = vec![Box::new(|tx| {
                tx.send(AppEvent::OpenAllModelsPopup);
            })];

            let is_current = !items.iter().any(|item| item.is_current);
            let description = Some(format!(
                "Choose a specific model and reasoning level (current: {current_label})"
            ));

            items.push(SelectionItem {
                name: "All models".to_string(),
                description,
                is_current,
                actions,
                dismiss_on_select: true,
                ..Default::default()
            });
        }

        let header = self.model_menu_header(
            "Select Model",
            "Pick a quick auto mode or browse all models.",
        );
        self.show_model_selection_view(
            model_ids,
            SelectionViewParams {
                view_id: Some(MODEL_SELECTION_VIEW_ID),
                items,
                header,
                ..SelectionViewParams::picker()
            },
        );
    }

    pub(super) fn is_auto_model(model: &str) -> bool {
        model.starts_with("codex-auto-")
    }

    #[allow(dead_code)]
    fn auto_model_order(model: &str) -> usize {
        match model {
            "codex-auto-fast" => 0,
            "codex-auto-balanced" => 1,
            "codex-auto-thorough" => 2,
            _ => 3,
        }
    }

    pub(crate) fn open_all_models_popup(&mut self) {
        if self.restrict_model_picker_to_luna_reserve() {
            self.open_luna_reserve_model_popup(
                self.model_catalog.try_list_models().unwrap_or_default(),
                ALL_MODELS_SELECTION_VIEW_ID,
            );
            return;
        }
        let presets = self
            .model_catalog
            .try_list_models()
            .unwrap_or_default()
            .into_iter()
            .filter(|preset| preset.show_in_picker && !Self::is_auto_model(&preset.model))
            .collect();
        self.open_all_models_popup_with_view_id(presets, ALL_MODELS_SELECTION_VIEW_ID);
    }

    fn open_all_models_popup_with_view_id(
        &mut self,
        presets: Vec<ModelPreset>,
        view_id: &'static str,
    ) {
        if presets.is_empty() {
            self.bottom_pane.dismiss_view_by_id(view_id);
            self.add_info_message(
                "No additional models are available right now.".to_string(),
                /*hint*/ None,
            );
            return;
        }

        let mut items: Vec<SelectionItem> = Vec::new();
        let model_ids = presets.iter().map(|preset| preset.model.clone()).collect();
        for preset in presets.into_iter() {
            let description =
                (!preset.description.is_empty()).then_some(preset.description.to_string());
            let is_current = preset.model.as_str() == self.current_model();
            let direct_effort = match preset.supported_reasoning_efforts.as_slice() {
                [] => Some(preset.default_reasoning_effort.clone()),
                [option] => Some(option.effort.clone()),
                _ => None,
            }
            .filter(|effort| !Self::is_advanced_reasoning_effort(effort));
            let single_supported_effort = direct_effort.is_some();
            let preset_for_action = preset.clone();
            let actions: Vec<SelectionAction> = vec![Box::new(move |tx| {
                let preset_for_event = preset_for_action.clone();
                tx.send(AppEvent::OpenReasoningPopup {
                    model: preset_for_event,
                });
            })];
            items.push(SelectionItem {
                name: preset.display_name.clone(),
                description,
                is_current,
                is_default: preset.is_default,
                secondary_action: direct_effort.and_then(|effort| {
                    self.session_model_selection_action(preset.model.clone(), Some(effort))
                }),
                actions,
                dismiss_on_select: single_supported_effort,
                dismiss_parent_on_child_accept: !single_supported_effort,
                ..Default::default()
            });
        }

        let header = self.model_menu_header("Select Model and Effort", "");
        self.show_model_selection_view(
            model_ids,
            SelectionViewParams {
                view_id: Some(view_id),
                items,
                header,
                ..SelectionViewParams::picker()
            },
        );
    }

    fn model_selection_actions(
        &self,
        model_for_action: String,
        effort_for_action: Option<ReasoningEffortConfig>,
        should_prompt_plan_mode_scope: bool,
    ) -> Vec<SelectionAction> {
        let warning = effort_for_action
            .as_ref()
            .and_then(|effort| self.ultra_reasoning_concurrency_warning(effort));
        let thread_id = self.thread_id();
        let sparkle_thread = self.sparkle_thread_for_picker_action(&model_for_action);
        vec![Box::new(move |tx| {
            if model_for_action == LUNA_RESERVE_MODEL {
                // Reserve is temporary: update the active task without persisting a model default.
                if let Some(thread_id) = thread_id {
                    tx.send(AppEvent::UpdateLunaReserveReasoning {
                        thread_id,
                        effort: effort_for_action.clone(),
                    });
                }
            } else if effort_for_action == Some(ReasoningEffortConfig::Ultra) {
                tx.send(
                    AstraModelPickerAction::ApplyAdvancedReasoning {
                        effort: ReasoningEffortConfig::Ultra,
                    }
                    .into_picker_event(sparkle_thread, model_for_action.clone()),
                );
            } else if should_prompt_plan_mode_scope {
                tx.send(AppEvent::OpenPlanReasoningScopePrompt {
                    model: model_for_action.clone(),
                    effort: effort_for_action.clone(),
                });
            } else {
                tx.send(
                    AstraModelPickerAction::UpdateModel
                        .into_picker_event(sparkle_thread, model_for_action.clone()),
                );
                tx.send(AppEvent::UpdateReasoningEffort(effort_for_action.clone()));
                tx.send(AppEvent::PersistModelSelection {
                    model: model_for_action.clone(),
                    effort: effort_for_action.clone(),
                });
            }
            if let Some(warning) = warning.clone() {
                tx.send(AppEvent::InsertHistoryCell(Box::new(
                    history_cell::new_warning_event(warning),
                )));
            }
        })]
    }

    fn should_prompt_plan_mode_reasoning_scope(
        &self,
        selected_model: &str,
        selected_effort: Option<ReasoningEffortConfig>,
    ) -> bool {
        if !self.collaboration_modes_enabled()
            || selected_model == LUNA_RESERVE_MODEL
            || self.active_mode_kind() != ModeKind::Plan
            || selected_model != self.current_model()
        {
            return false;
        }

        // Prompt whenever the selection is not a true no-op for both:
        // 1) the active Plan-mode effective reasoning, and
        // 2) the stored global defaults that would be updated by the fallback path.
        selected_effort != self.effective_reasoning_effort()
            || selected_model != self.current_collaboration_mode.model()
            || selected_effort != self.current_collaboration_mode.reasoning_effort()
    }

    pub(crate) fn open_plan_reasoning_scope_prompt(
        &mut self,
        model: String,
        effort: Option<ReasoningEffortConfig>,
    ) {
        let reasoning_phrase = match effort.as_ref() {
            Some(ReasoningEffortConfig::None) => "no reasoning".to_string(),
            Some(selected_effort) => {
                format!(
                    "{} reasoning",
                    Self::reasoning_effort_sentence_label(selected_effort)
                )
            }
            None => "the selected reasoning".to_string(),
        };
        let plan_only_description = format!("Always use {reasoning_phrase} in Plan mode.");
        let plan_reasoning_source = if let Some(plan_override) =
            self.config.plan_mode_reasoning_effort.as_ref()
        {
            format!(
                "user-chosen Plan override ({})",
                Self::reasoning_effort_sentence_label(plan_override)
            )
        } else if let Some(plan_mask) = collaboration_modes::plan_mask(self.model_catalog.as_ref())
        {
            match plan_mask
                .reasoning_effort
                .as_ref()
                .and_then(|effort| effort.as_ref())
            {
                Some(plan_effort) => format!(
                    "built-in Plan default ({})",
                    Self::reasoning_effort_sentence_label(plan_effort)
                ),
                None => "built-in Plan default (no reasoning)".to_string(),
            }
        } else {
            "built-in Plan default".to_string()
        };
        let all_modes_description = format!(
            "Set the global default reasoning level and the Plan mode override. This replaces the current {plan_reasoning_source}."
        );
        let subtitle = format!("Choose where to apply {reasoning_phrase}.");
        let warning = effort
            .as_ref()
            .and_then(|effort| self.ultra_reasoning_concurrency_warning(effort));
        let sparkle_thread = self.sparkle_thread_for_picker_action(&model);

        let plan_only_actions: Vec<SelectionAction> = vec![Box::new({
            let model = model.clone();
            let effort = effort.clone();
            let warning = warning.clone();
            move |tx| {
                tx.send(
                    AstraModelPickerAction::UpdateModel
                        .into_picker_event(sparkle_thread, model.clone()),
                );
                tx.send(AppEvent::UpdatePlanModeReasoningEffort(effort.clone()));
                tx.send(AppEvent::PersistPlanModeReasoningEffort(effort.clone()));
                if let Some(warning) = warning.clone() {
                    tx.send(AppEvent::InsertHistoryCell(Box::new(
                        history_cell::new_warning_event(warning),
                    )));
                }
            }
        })];
        let all_modes_actions: Vec<SelectionAction> = vec![Box::new(move |tx| {
            tx.send(
                AstraModelPickerAction::UpdateModel
                    .into_picker_event(sparkle_thread, model.clone()),
            );
            tx.send(AppEvent::UpdateReasoningEffort(effort.clone()));
            tx.send(AppEvent::UpdatePlanModeReasoningEffort(effort.clone()));
            tx.send(AppEvent::PersistPlanModeReasoningEffort(effort.clone()));
            tx.send(AppEvent::PersistModelSelection {
                model: model.clone(),
                effort: effort.clone(),
            });
            if let Some(warning) = warning.clone() {
                tx.send(AppEvent::InsertHistoryCell(Box::new(
                    history_cell::new_warning_event(warning),
                )));
            }
        })];

        self.bottom_pane.show_selection_view(SelectionViewParams {
            title: Some(PLAN_MODE_REASONING_SCOPE_TITLE.to_string()),
            subtitle: Some(subtitle),
            items: vec![
                SelectionItem {
                    name: PLAN_MODE_REASONING_SCOPE_PLAN_ONLY.to_string(),
                    description: Some(plan_only_description),
                    actions: plan_only_actions,
                    dismiss_on_select: true,
                    ..Default::default()
                },
                SelectionItem {
                    name: PLAN_MODE_REASONING_SCOPE_ALL_MODES.to_string(),
                    description: Some(all_modes_description),
                    actions: all_modes_actions,
                    dismiss_on_select: true,
                    ..Default::default()
                },
            ],
            ..SelectionViewParams::picker()
        });
        self.notify(Notification::PlanModePrompt {
            title: PLAN_MODE_REASONING_SCOPE_TITLE.to_string(),
        });
    }

    /// Open a popup to choose the standard reasoning effort for the given model.
    ///
    /// Max and Ultra require an explicit second step so expensive efforts cannot
    /// be selected accidentally while moving through the normal effort scale.
    pub(crate) fn open_reasoning_popup(&mut self, preset: ModelPreset) {
        let default_effort = preset.default_reasoning_effort.clone();
        let supported = &preset.supported_reasoning_efforts;
        let in_plan_mode =
            self.collaboration_modes_enabled() && self.active_mode_kind() == ModeKind::Plan;

        let warn_effort = if supported
            .iter()
            .any(|option| option.effort == ReasoningEffortConfig::XHigh)
        {
            Some(ReasoningEffortConfig::XHigh)
        } else if supported
            .iter()
            .any(|option| option.effort == ReasoningEffortConfig::High)
        {
            Some(ReasoningEffortConfig::High)
        } else {
            None
        };
        let warning_text = warn_effort.as_ref().map(|effort| {
            let effort_label = Self::reasoning_effort_label(effort);
            format!("⚠ {effort_label} reasoning effort can quickly consume Plus plan rate limits.")
        });
        let warn_for_model = preset.model.starts_with("gpt-5.1-codex")
            || preset.model.starts_with("gpt-5.1-codex-max")
            || preset.model.starts_with("gpt-5.2");

        let mut all_choices: Vec<ReasoningEffortConfig> = supported
            .iter()
            .map(|option| option.effort.clone())
            .collect();
        if all_choices.is_empty() {
            all_choices.push(default_effort.clone());
        }
        let (choices, advanced_choices): (Vec<_>, Vec<_>) = all_choices
            .into_iter()
            .partition(|effort| !Self::is_advanced_reasoning_effort(effort));

        if choices.len() == 1 && advanced_choices.is_empty() {
            let selected_effort = choices.first().cloned();
            let selected_model = preset.model;
            if self
                .should_prompt_plan_mode_reasoning_scope(&selected_model, selected_effort.clone())
            {
                self.app_event_tx
                    .send(AppEvent::OpenPlanReasoningScopePrompt {
                        model: selected_model,
                        effort: selected_effort,
                    });
            } else {
                self.apply_model_and_effort(selected_model, selected_effort);
            }
            return;
        }

        let default_choice = choices
            .contains(&default_effort)
            .then(|| default_effort.clone());

        let model_slug = preset.model.to_string();
        let model_label = preset.display_name.clone();
        let is_current_model = self.current_model() == preset.model.as_str();
        let highlight_choice = if is_current_model {
            if in_plan_mode {
                self.config
                    .plan_mode_reasoning_effort
                    .clone()
                    .or_else(|| self.effective_reasoning_effort())
            } else {
                self.effective_reasoning_effort()
            }
        } else {
            default_choice.clone().or_else(|| choices.first().cloned())
        };
        let selection_choice = highlight_choice.clone().or_else(|| default_choice.clone());
        let initial_selected_idx = choices
            .iter()
            .position(|choice| Some(choice) == selection_choice.as_ref());
        let mut items: Vec<SelectionItem> = Vec::new();
        for choice in choices.iter() {
            let effort = choice.clone();
            let mut effort_label = Self::reasoning_effort_label(&effort);
            if Some(choice) == default_choice.as_ref() {
                effort_label.push_str(" (default)");
            }

            let description = supported
                .iter()
                .find(|option| option.effort == effort)
                .map(|option| option.description.to_string())
                .filter(|text| !text.is_empty());

            let show_warning = warn_for_model && warn_effort.as_ref() == Some(&effort);
            let selected_description = if show_warning {
                warning_text.as_ref().map(|warning_message| {
                    description.as_ref().map_or_else(
                        || warning_message.clone(),
                        |d| format!("{d}\n{warning_message}"),
                    )
                })
            } else {
                None
            };

            let choice_effort = Some(effort);
            let should_prompt_plan_mode_scope = self.should_prompt_plan_mode_reasoning_scope(
                model_slug.as_str(),
                choice_effort.clone(),
            );
            let actions = self.model_selection_actions(
                model_slug.clone(),
                choice_effort.clone(),
                should_prompt_plan_mode_scope,
            );

            items.push(SelectionItem {
                name: effort_label,
                description,
                selected_description,
                is_current: is_current_model && Some(choice) == highlight_choice.as_ref(),
                secondary_action: self
                    .session_model_selection_action(model_slug.clone(), choice_effort),
                actions,
                dismiss_on_select: true,
                ..Default::default()
            });
        }

        if !advanced_choices.is_empty() {
            let advanced_label = advanced_choices
                .iter()
                .map(Self::reasoning_effort_label)
                .collect::<Vec<_>>()
                .join(" and ");
            let verb = if advanced_choices.len() == 1 {
                "consumes"
            } else {
                "consume"
            };
            let preset_for_action = preset;
            let actions: Vec<SelectionAction> = vec![Box::new(move |tx| {
                tx.send(AppEvent::OpenAdvancedReasoningPopup {
                    model: preset_for_action.clone(),
                });
            })];
            items.push(SelectionItem {
                name: "More reasoning…".to_string(),
                description: Some(format!("{advanced_label} {verb} usage limits faster")),
                is_current: is_current_model
                    && highlight_choice
                        .as_ref()
                        .is_some_and(Self::is_advanced_reasoning_effort),
                actions,
                dismiss_parent_on_child_accept: true,
                ..Default::default()
            });
        }

        let header = Paragraph::new(Line::from(
            format!("Select Reasoning Level for {model_label}").bold(),
        ))
        .wrap(Wrap { trim: false });

        self.bottom_pane.show_selection_view(SelectionViewParams {
            header: Box::new(header),
            items,
            initial_selected_idx,
            ..SelectionViewParams::picker()
        });
    }

    /// Open the explicit Max/Ultra effort picker for the given model.
    pub(crate) fn open_advanced_reasoning_popup(&mut self, preset: ModelPreset) {
        let mut choices = preset
            .supported_reasoning_efforts
            .iter()
            .map(|option| option.effort.clone())
            .filter(Self::is_advanced_reasoning_effort)
            .collect::<Vec<_>>();
        if choices.is_empty()
            && Self::is_advanced_reasoning_effort(&preset.default_reasoning_effort)
        {
            choices.push(preset.default_reasoning_effort.clone());
        }
        choices.sort_by_key(|effort| matches!(effort, ReasoningEffortConfig::Ultra));
        if choices.is_empty() {
            return;
        }

        let model_slug = preset.model.to_string();
        let is_current_model = self.current_model() == preset.model.as_str();
        let highlight_choice = is_current_model
            .then(|| self.effective_reasoning_effort())
            .flatten();
        let mut items = Vec::new();
        for effort in choices {
            let description = match &effort {
                ReasoningEffortConfig::Max => {
                    "For difficult problems when quality matters more than speed · higher usage"
                }
                ReasoningEffortConfig::Ultra => {
                    "For demanding work using multiple agents · highest usage"
                }
                _ => unreachable!("advanced choices are limited to Max and Ultra"),
            };
            let should_prompt_plan_mode_scope = self
                .should_prompt_plan_mode_reasoning_scope(model_slug.as_str(), Some(effort.clone()));
            let actions = self.model_selection_actions(
                model_slug.clone(),
                Some(effort.clone()),
                should_prompt_plan_mode_scope,
            );

            items.push(SelectionItem {
                name: Self::reasoning_effort_label(&effort),
                description: Some(description.to_string()),
                is_current: is_current_model && Some(&effort) == highlight_choice.as_ref(),
                secondary_action: self
                    .session_model_selection_action(model_slug.clone(), Some(effort.clone())),
                actions,
                dismiss_on_select: true,
                ..Default::default()
            });
        }

        let header = Paragraph::new(vec![
            Line::from("Advanced Reasoning".bold()),
            Line::from("⚠ Consumes usage limits faster".cyan()),
        ])
        .wrap(Wrap { trim: false });
        self.bottom_pane.show_selection_view(SelectionViewParams {
            header: Box::new(header),
            items,
            ..SelectionViewParams::picker()
        });
    }

    pub(super) fn is_advanced_reasoning_effort(effort: &ReasoningEffortConfig) -> bool {
        matches!(
            effort,
            ReasoningEffortConfig::Max | ReasoningEffortConfig::Ultra
        )
    }

    pub(super) fn reasoning_effort_label(effort: &ReasoningEffortConfig) -> String {
        match effort {
            ReasoningEffortConfig::None => "None".to_string(),
            ReasoningEffortConfig::Minimal => "Minimal".to_string(),
            ReasoningEffortConfig::Low => "Low".to_string(),
            ReasoningEffortConfig::Medium => "Medium".to_string(),
            ReasoningEffortConfig::High => "High".to_string(),
            ReasoningEffortConfig::XHigh => "Extra high".to_string(),
            ReasoningEffortConfig::Max => "Max".to_string(),
            ReasoningEffortConfig::Ultra => "Ultra".to_string(),
            ReasoningEffortConfig::Persistent => "Persistent".to_string(),
            ReasoningEffortConfig::Custom(value) => value.clone(),
        }
    }

    pub(super) fn reasoning_effort_sentence_label(effort: &ReasoningEffortConfig) -> String {
        match effort {
            ReasoningEffortConfig::Custom(value) => value.clone(),
            effort => Self::reasoning_effort_label(effort).to_lowercase(),
        }
    }

    pub(super) fn ultra_reasoning_concurrency_warning(
        &self,
        effort: &ReasoningEffortConfig,
    ) -> Option<String> {
        if effort != &ReasoningEffortConfig::Ultra {
            return None;
        }

        let max_threads = self
            .config
            .multi_agent_v2
            .max_concurrent_threads_per_session;
        if max_threads < ULTRA_REASONING_CONCURRENCY_WARNING_THRESHOLD {
            return None;
        }

        let max_subagents = max_threads.saturating_sub(1);
        Some(format!(
            "Ultra reasoning may proactively use multiple agents. This session is configured for \
             {max_threads} concurrent threads with up to {max_subagents} subagents which can \
             increase usage quickly. Consider setting \
             features.multi_agent_v2.max_concurrent_threads_per_session below 8."
        ))
    }

    fn apply_model_and_effort(&self, model: String, effort: Option<ReasoningEffortConfig>) {
        for action in self
            .model_selection_actions(model, effort, /*should_prompt_plan_mode_scope*/ false)
        {
            action(&self.app_event_tx);
        }
    }
    pub(crate) fn open_model_popup_for_provider(
        &mut self,
        provider_id: String,
        provider_name: String,
        mut presets: Vec<ModelPreset>,
    ) {
        if self.bottom_pane.active_view_id() != Some(MODEL_SELECTION_VIEW_ID) {
            return;
        }

        presets.retain(|preset| preset.show_in_picker);
        presets.sort_by(|left, right| left.model.cmp(&right.model));

        let mut items = presets
            .into_iter()
            .map(|preset| {
                let provider_id_for_action = provider_id.clone();
                let provider_name_for_action = provider_name.clone();
                let single_effort = single_supported_reasoning_effort(&preset);
                let actions: Vec<SelectionAction> = if let Some(effort) = single_effort {
                    let model = preset.model.clone();
                    vec![Box::new(move |tx| {
                        tx.send(AppEvent::LoadHarnesses {
                            provider_id: provider_id_for_action.clone(),
                            provider_name: provider_name_for_action.clone(),
                            model: model.clone(),
                            effort: effort.clone(),
                        });
                    })]
                } else {
                    let model = preset.clone();
                    vec![Box::new(move |tx| {
                        tx.send(AppEvent::OpenReasoningPopupForProvider {
                            provider_id: provider_id_for_action.clone(),
                            provider_name: provider_name_for_action.clone(),
                            model: model.clone(),
                        });
                    })]
                };
                SelectionItem {
                    name: preset.model.clone(),
                    description: (!preset.description.is_empty())
                        .then_some(preset.description.clone()),
                    is_current: provider_id == self.config.model_provider_id
                        && preset.model == self.current_model(),
                    is_default: preset.is_default,
                    actions,
                    dismiss_on_select: false,
                    dismiss_parent_on_child_accept: true,
                    search_value: Some(format!(
                        "{} {} {}",
                        preset.model, preset.display_name, preset.description
                    )),
                    ..Default::default()
                }
            })
            .collect::<Vec<_>>();

        let custom_provider_id = provider_id;
        let custom_provider_name = provider_name.clone();
        items.push(SelectionItem {
            name: "Custom model name".to_string(),
            description: Some("Type a model id that this provider accepts.".to_string()),
            actions: vec![Box::new(move |tx| {
                tx.send(AppEvent::OpenCustomProviderModelPrompt {
                    provider_id: custom_provider_id.clone(),
                    provider_name: custom_provider_name.clone(),
                    initial_text: None,
                });
            })],
            dismiss_on_select: false,
            dismiss_parent_on_child_accept: true,
            search_value: Some("custom manual typed model".to_string()),
            ..Default::default()
        });

        let header = self.model_menu_header(
            &format!("Select Model for {provider_name}"),
            "Choose a listed model or enter a custom model id.",
        );
        let _ = self.bottom_pane.replace_selection_view_if_active(
            MODEL_SELECTION_VIEW_ID,
            SelectionViewParams {
                view_id: Some(MODEL_SELECTION_VIEW_ID),
                footer_hint: Some("Type to filter • Enter to select • Esc to dismiss".into()),
                items,
                header,
                is_searchable: true,
                search_placeholder: Some("Filter models".to_string()),
                ..Default::default()
            },
        );
    }

    pub(crate) fn open_custom_model_prompt_for_provider(
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
            Some("This will start a new chat with the selected provider.".to_string()),
            Box::new(move |model: String| {
                let model = model.trim().to_string();
                if !model.is_empty() {
                    tx.send(AppEvent::LoadHarnesses {
                        provider_id: provider_id.clone(),
                        provider_name: provider_name.clone(),
                        model,
                        effort: None,
                    });
                }
            }),
        );
        self.bottom_pane.show_view(Box::new(view));
    }

    pub(crate) fn open_reasoning_popup_for_provider(
        &mut self,
        provider_id: String,
        provider_name: String,
        preset: ModelPreset,
    ) {
        let default_effort = preset.default_reasoning_effort;
        if preset.supported_reasoning_efforts.is_empty() {
            self.app_event_tx.send(AppEvent::LoadHarnesses {
                provider_id,
                provider_name,
                model: preset.model,
                effort: None,
            });
            return;
        }

        let choices = preset
            .supported_reasoning_efforts
            .into_iter()
            .map(|option| {
                let effort = option.effort;
                let mut label = Self::reasoning_effort_label(&effort);
                if effort == default_effort {
                    label.push_str(" (default)");
                }
                (label, effort, option.description)
            })
            .collect::<Vec<_>>();
        let initial_selected_idx = choices
            .iter()
            .position(|(_, effort, _)| effort == &default_effort);
        let items = choices
            .into_iter()
            .map(|(label, effort, description)| {
                let action_provider_id = provider_id.clone();
                let action_provider_name = provider_name.clone();
                let model = preset.model.clone();
                SelectionItem {
                    name: label,
                    description: (!description.is_empty()).then_some(description),
                    actions: vec![Box::new(move |tx| {
                        tx.send(AppEvent::LoadHarnesses {
                            provider_id: action_provider_id.clone(),
                            provider_name: action_provider_name.clone(),
                            model: model.clone(),
                            effort: Some(effort.clone()),
                        });
                    })],
                    dismiss_on_select: false,
                    dismiss_parent_on_child_accept: true,
                    ..Default::default()
                }
            })
            .collect();

        let header = self.model_menu_header(
            &format!("Select Reasoning Level for {}", preset.model),
            &format!("{provider_name} will start a new chat with this selection."),
        );
        self.bottom_pane.show_selection_view(SelectionViewParams {
            header,
            footer_hint: Some(standard_popup_hint_line()),
            items,
            initial_selected_idx,
            ..Default::default()
        });
    }

    pub(crate) fn open_harness_popup_for_provider(
        &mut self,
        provider_id: String,
        provider_name: String,
        model: String,
        effort: Option<ReasoningEffortConfig>,
        harnesses: Vec<InterpreterHarness>,
    ) {
        let items = harnesses
            .into_iter()
            .map(|harness| {
                let action_provider_id = provider_id.clone();
                let action_provider_name = provider_name.clone();
                let action_model = model.clone();
                let action_effort = effort.clone();
                let action_harness = harness.id.clone();
                SelectionItem {
                    name: harness.label,
                    description: Some(harness.description),
                    is_default: harness.is_recommended,
                    actions: vec![Box::new(move |tx| {
                        tx.send(AppEvent::PersistProviderModelSelection {
                            provider_id: action_provider_id.clone(),
                            provider_name: action_provider_name.clone(),
                            model: action_model.clone(),
                            effort: action_effort.clone(),
                            harness: action_harness.clone(),
                        });
                    })],
                    dismiss_on_select: true,
                    dismiss_parent_on_child_accept: true,
                    ..Default::default()
                }
            })
            .collect();
        let header = self.model_menu_header(
            "Select Tool Harness",
            &format!("{provider_name} / {model} will start a new chat."),
        );
        let _ = self.bottom_pane.replace_selection_view_if_active(
            HARNESS_SELECTION_VIEW_ID,
            SelectionViewParams {
                view_id: Some(HARNESS_SELECTION_VIEW_ID),
                footer_hint: Some(standard_popup_hint_line()),
                items,
                header,
                ..Default::default()
            },
        );
    }

    pub(crate) fn open_harness_loading_popup(&mut self, provider_name: &str, model: &str) {
        self.bottom_pane.show_selection_view(SelectionViewParams {
            view_id: Some(HARNESS_SELECTION_VIEW_ID),
            title: Some("Select Tool Harness".to_string()),
            subtitle: Some(format!(
                "Loading compatible harnesses for {provider_name} / {model}..."
            )),
            items: vec![SelectionItem {
                name: "Loading harnesses...".to_string(),
                is_disabled: true,
                ..Default::default()
            }],
            ..Default::default()
        });
    }

    pub(crate) fn on_harnesses_error(&mut self, message: String) {
        if self
            .bottom_pane
            .dismiss_active_view_if_id(HARNESS_SELECTION_VIEW_ID)
        {
            self.add_error_message(message);
        }
    }
}

fn single_supported_reasoning_effort(
    preset: &ModelPreset,
) -> Option<Option<ReasoningEffortConfig>> {
    match preset.supported_reasoning_efforts.as_slice() {
        [] => Some(None),
        [only] => Some(Some(only.effort.clone())),
        _ => None,
    }
}
