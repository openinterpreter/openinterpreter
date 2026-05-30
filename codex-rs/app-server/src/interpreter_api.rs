//! Handlers for the `interpreter*` app-server methods.
//!
//! These power the non-interactive "pick a provider, pick a model, pick a
//! harness" flow over JSON-RPC. The dispatch arms in
//! [`crate::codex_message_processor`] delegate to these free functions.

use std::sync::Arc;

use codex_app_server_protocol::ConfigBatchWriteParams;
use codex_app_server_protocol::ConfigEdit;
use codex_app_server_protocol::InterpreterHarness;
use codex_app_server_protocol::InterpreterHarnessListParams;
use codex_app_server_protocol::InterpreterHarnessListResponse;
use codex_app_server_protocol::InterpreterHarnessSetParams;
use codex_app_server_protocol::InterpreterHarnessSetResponse;
use codex_app_server_protocol::InterpreterModelListParams;
use codex_app_server_protocol::InterpreterModelListResponse;
use codex_app_server_protocol::InterpreterModelSetParams;
use codex_app_server_protocol::InterpreterModelSetResponse;
use codex_app_server_protocol::InterpreterProvider;
use codex_app_server_protocol::InterpreterProviderKind;
use codex_app_server_protocol::InterpreterProviderListParams;
use codex_app_server_protocol::InterpreterProviderListResponse;
use codex_app_server_protocol::InterpreterProviderSetParams;
use codex_app_server_protocol::InterpreterProviderSetResponse;
use codex_app_server_protocol::JSONRPCErrorError;
use codex_app_server_protocol::ProviderReadinessDto;
use codex_app_server_protocol::WireApiDto;
use codex_core::config::Config;
use codex_core::config::edit::ConfigEditsBuilder;
use codex_login::AuthManager;
use codex_model_provider_info::WireApi;
use codex_model_provider_info::harness_selection::harness_choices_for_provider_model;
use codex_model_provider_info::provider_selection::ProviderChoice;
use codex_model_provider_info::provider_selection::ProviderChoiceAction;
use codex_model_provider_info::provider_selection::ProviderPreset;
use codex_model_provider_info::provider_selection::ProviderPresetQuickAddAction;
use codex_model_provider_info::provider_selection::ProviderReadiness;
use codex_model_provider_info::provider_selection::ProviderReadinessSnapshot;
use codex_model_provider_info::provider_selection::model_picker_provider_choices_with_snapshot;
use codex_model_provider_info::provider_selection::provider_preset_by_id;
use codex_model_provider_info::provider_selection::set_path;

use crate::config_manager::ConfigManager;
use crate::error_code::INTERNAL_ERROR_CODE;
use crate::error_code::INVALID_PARAMS_ERROR_CODE;
use crate::models::supported_models;
use crate::models::supported_models_for_provider;

fn map_wire(wire_api: WireApi) -> WireApiDto {
    match wire_api {
        WireApi::Responses => WireApiDto::Responses,
        WireApi::Chat => WireApiDto::Chat,
        WireApi::Messages => WireApiDto::Messages,
    }
}

fn map_readiness(readiness: ProviderReadiness) -> ProviderReadinessDto {
    match readiness {
        ProviderReadiness::LoggedIn => ProviderReadinessDto::LoggedIn,
        ProviderReadiness::Ready => ProviderReadinessDto::Ready,
        ProviderReadiness::Installed => ProviderReadinessDto::Installed,
        ProviderReadiness::NeedsSetup => ProviderReadinessDto::NeedsSetup,
    }
}

/// Constructors for the JSON-RPC errors these handlers return, so call sites read
/// `JSONRPCErrorError::internal(msg)` instead of a bare struct literal. Defined as
/// a fork-local extension trait (reusing `error_code`'s constants) because the
/// type lives in `codex-app-server-protocol`, which cannot depend on this crate to
/// host the constructors itself.
trait JsonRpcErrorExt {
    fn internal(message: String) -> Self;
    fn invalid_params(message: String) -> Self;
}

impl JsonRpcErrorExt for JSONRPCErrorError {
    fn internal(message: String) -> Self {
        Self {
            code: INTERNAL_ERROR_CODE,
            data: None,
            message,
        }
    }

    fn invalid_params(message: String) -> Self {
        Self {
            code: INVALID_PARAMS_ERROR_CODE,
            data: None,
            message,
        }
    }
}

/// List providers for the `/model` picker, identical to the TUI's list.
///
/// Built from `codex_model_provider_info::provider_selection`'s
/// `model_picker_provider_choices_with_snapshot` so this matches the TUI
/// exactly: configured providers union quick-add presets, with readiness
/// labels, the OpenAI ChatGPT-vs-API-key split, and the readiness sort.
///
/// Each `ProviderChoice` maps to an `InterpreterProvider`. `kind` is `Existing`
/// for configured providers and `QuickAdd` for presets; `configured` mirrors
/// `kind == Existing`; `is_default` is true when the entry id equals
/// `config.model_provider_id`. `base_url`/`wire_api`/`env_key` are populated for
/// configured providers from `config.model_providers` and left unset for
/// presets. `include_unconfigured` defaults to true; when `Some(false)`, the
/// quick-add (preset) entries are dropped, leaving only configured providers.
pub fn list_providers(
    config: &Config,
    params: InterpreterProviderListParams,
) -> InterpreterProviderListResponse {
    let include_unconfigured = params.include_unconfigured.unwrap_or(true);
    let snapshot = ProviderReadinessSnapshot::from_system(config.codex_home.as_path());
    let choices = model_picker_provider_choices_with_snapshot(
        &config.model_providers,
        &config.model_provider_id,
        &snapshot,
    );

    let data: Vec<InterpreterProvider> = choices
        .into_iter()
        .filter(|choice| {
            include_unconfigured || matches!(choice.action, ProviderChoiceAction::Existing)
        })
        .map(|choice| {
            let ProviderChoice {
                id,
                name,
                description,
                readiness,
                is_current,
                starts_new_chat,
                action,
            } = choice;
            let kind = match action {
                ProviderChoiceAction::Existing => InterpreterProviderKind::Existing,
                ProviderChoiceAction::QuickAdd(_) => InterpreterProviderKind::QuickAdd,
            };
            let configured = matches!(kind, InterpreterProviderKind::Existing);
            // Carry concrete connection details for already-configured providers
            // only; presets do not correspond to a `config.model_providers` entry.
            let provider = configured
                .then(|| config.model_providers.get(&id))
                .flatten();
            InterpreterProvider {
                is_default: id == config.model_provider_id,
                id,
                name,
                description,
                readiness: map_readiness(readiness),
                kind,
                is_current,
                starts_new_chat,
                base_url: provider.and_then(|p| p.base_url.clone()),
                wire_api: provider.map(|p| map_wire(p.wire_api)),
                env_key: provider.and_then(|p| p.env_key.clone()),
                configured,
            }
        })
        .collect();

    InterpreterProviderListResponse { data }
}

/// List the models available for a provider. Performs network I/O.
///
/// When `model_provider` is set, lists that provider's models; otherwise lists
/// the active provider's models. `include_hidden` defaults to false.
pub async fn list_models(
    config: &Config,
    auth_manager: Arc<AuthManager>,
    params: InterpreterModelListParams,
) -> Result<InterpreterModelListResponse, JSONRPCErrorError> {
    let InterpreterModelListParams {
        model_provider,
        include_hidden,
    } = params;
    let include_hidden = include_hidden.unwrap_or(false);
    let data = match model_provider {
        Some(provider_id) => supported_models_for_provider(
            config,
            auth_manager,
            provider_id.as_str(),
            include_hidden,
        )
        .await
        .map_err(JSONRPCErrorError::invalid_params)?,
        None => supported_models(config, auth_manager, include_hidden).await,
    };
    Ok(InterpreterModelListResponse { data })
}

/// List the harness choices compatible with a provider/model.
///
/// Provider details (name, base URL, wire API) come from
/// `config.model_providers` when configured; otherwise the bundled catalog is
/// consulted internally by `harness_choices_for_provider_model`.
pub fn list_harnesses(
    config: &Config,
    params: InterpreterHarnessListParams,
) -> InterpreterHarnessListResponse {
    let InterpreterHarnessListParams { provider_id, model } = params;
    let provider = config.model_providers.get(&provider_id);
    let choices = harness_choices_for_provider_model(
        &provider_id,
        provider.map(|p| p.name.as_str()),
        provider.and_then(|p| p.base_url.as_deref()),
        provider.map(|p| p.wire_api),
        model.as_deref(),
    );
    let data = choices
        .into_iter()
        .map(|choice| InterpreterHarness {
            id: choice.stored,
            label: choice.label,
            description: choice.description,
            is_recommended: choice.is_recommended,
        })
        .collect();
    InterpreterHarnessListResponse { data }
}

/// Persist the selected provider to config (affects future turns).
///
/// When `provider_id` names a quick-add preset that is not yet configured, the
/// preset's provider definition is written before it is selected, mirroring the
/// TUI's selection flow so the provider is actually usable. Presets that require
/// an API key with none in the environment must supply `api_key`. The synthetic
/// `openai::chatgpt` picker id is normalized to its base provider id.
pub async fn set_provider(
    config: &Config,
    config_manager: &ConfigManager,
    params: InterpreterProviderSetParams,
) -> Result<InterpreterProviderSetResponse, JSONRPCErrorError> {
    let InterpreterProviderSetParams {
        provider_id,
        profile,
        api_key,
    } = params;
    let provider_id = provider_id
        .split_once("::")
        .map(|(base, _)| base.to_string())
        .unwrap_or(provider_id);

    let mut edits: Vec<ConfigEdit> = Vec::new();
    if !config.model_providers.contains_key(&provider_id)
        && let Some(preset) = provider_preset_by_id(&provider_id)
    {
        edits.extend(preset_definition_edits(
            &preset,
            &provider_id,
            api_key.as_deref(),
        )?);
    }

    // Select the provider, scoped to the profile when one is given (the same key
    // `ConfigEditsBuilder::set_model_provider` writes).
    let select_key = match profile.as_deref() {
        Some(profile) => format!("profiles.{profile}.model_provider"),
        None => "model_provider".to_string(),
    };
    edits.push(set_path(select_key, serde_json::json!(provider_id)));

    config_manager
        .batch_write(ConfigBatchWriteParams {
            edits,
            file_path: None,
            expected_version: None,
            reload_user_config: true,
        })
        .await
        .map_err(|err| {
            JSONRPCErrorError::internal(format!("failed to set model provider: {err}"))
        })?;
    Ok(InterpreterProviderSetResponse {})
}

/// Provider-definition edits for a not-yet-configured quick-add preset, mirroring
/// what the TUI writes when the same preset is chosen.
fn preset_definition_edits(
    preset: &ProviderPreset,
    provider_id: &str,
    api_key: Option<&str>,
) -> Result<Vec<ConfigEdit>, JSONRPCErrorError> {
    match preset.quick_add_action() {
        // Built-in/already-resolvable presets (Ollama, ChatGPT, env-key present):
        // the edits may be empty, in which case selection alone suffices.
        Some(ProviderPresetQuickAddAction::WriteEdits(edits)) => Ok(edits),
        // The preset needs an API key and none was found in the environment.
        Some(ProviderPresetQuickAddAction::PromptForApiKey) => {
            let api_key = api_key
                .map(str::trim)
                .filter(|key| !key.is_empty())
                .ok_or_else(|| {
                    JSONRPCErrorError::invalid_params(format!(
                        "provider `{provider_id}` requires an API key; pass `api_key`"
                    ))
                })?;
            Ok(preset.provider_definition_edits(
                &preset.configured_provider_id(None),
                &preset.configured_provider_name(None),
                &preset.base_url,
                api_key,
                /*api_key_prefilled_from_env*/ false,
            ))
        }
        // Custom (base-URL-editable) presets cannot be added from an id alone.
        None => Err(JSONRPCErrorError::invalid_params(format!(
            "provider `{provider_id}` cannot be added automatically; configure it explicitly"
        ))),
    }
}

/// Persist the selected model (and optional reasoning effort) to config.
pub async fn set_model(
    config: &Config,
    params: InterpreterModelSetParams,
) -> Result<InterpreterModelSetResponse, JSONRPCErrorError> {
    let InterpreterModelSetParams {
        model,
        reasoning_effort,
        profile,
    } = params;
    ConfigEditsBuilder::new(&config.codex_home)
        .with_profile(profile.as_deref())
        .set_model(Some(&model), reasoning_effort)
        .apply()
        .await
        .map_err(|err| JSONRPCErrorError::internal(format!("failed to set model: {err}")))?;
    Ok(InterpreterModelSetResponse {})
}

/// Persist the selected harness to config. `harness == None` selects native.
pub async fn set_harness(
    config: &Config,
    params: InterpreterHarnessSetParams,
) -> Result<InterpreterHarnessSetResponse, JSONRPCErrorError> {
    let InterpreterHarnessSetParams { harness, profile } = params;
    ConfigEditsBuilder::new(&config.codex_home)
        .with_profile(profile.as_deref())
        .set_harness(harness.as_deref())
        .apply()
        .await
        .map_err(|err| JSONRPCErrorError::internal(format!("failed to set harness: {err}")))?;
    Ok(InterpreterHarnessSetResponse {})
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_core::config::Config;
    use codex_core::config::ConfigBuilder;
    use codex_model_provider_info::ModelProviderInfo;
    use std::collections::BTreeSet;
    use tempfile::tempdir;

    async fn empty_config() -> Config {
        let temp_dir = tempdir().expect("tempdir");
        ConfigBuilder::default()
            .codex_home(temp_dir.path().to_path_buf())
            .build()
            .await
            .expect("config")
    }

    fn provider(name: &str, wire_api: WireApi) -> ModelProviderInfo {
        ModelProviderInfo {
            name: name.to_string(),
            wire_api,
            ..Default::default()
        }
    }

    fn provider_ids(response: &InterpreterProviderListResponse) -> Vec<&str> {
        response.data.iter().map(|p| p.id.as_str()).collect()
    }

    fn harness_ids(response: &InterpreterHarnessListResponse) -> BTreeSet<Option<String>> {
        response.data.iter().map(|h| h.id.clone()).collect()
    }

    async fn list_harnesses_for_wire(wire_api: WireApi) -> InterpreterHarnessListResponse {
        // Use a provider id that is absent from the bundled catalog (and no
        // base URL) so `harness_choices_for_provider_model` falls back to the
        // configured `wire_api` rather than a catalog-derived one. A neutral
        // name keeps the recommendation deterministic (no model-family match).
        let provider_id = "custom-provider";
        let mut config = empty_config().await;
        config
            .model_providers
            .insert(provider_id.to_string(), provider("Custom", wire_api));
        list_harnesses(
            &config,
            InterpreterHarnessListParams {
                provider_id: provider_id.to_string(),
                model: None,
            },
        )
    }

    #[tokio::test]
    async fn list_providers_marks_configured_and_single_default() {
        let mut config = empty_config().await;
        config
            .model_providers
            .insert("custom-a".to_string(), provider("Custom A", WireApi::Chat));
        config
            .model_providers
            .insert("custom-b".to_string(), provider("Custom B", WireApi::Chat));
        config.model_provider_id = "custom-a".to_string();

        let response = list_providers(
            &config,
            InterpreterProviderListParams {
                include_unconfigured: Some(false),
            },
        );

        // Dropping unconfigured entries leaves only Existing (configured) providers.
        assert!(
            response
                .data
                .iter()
                .all(|p| p.configured && matches!(p.kind, InterpreterProviderKind::Existing)),
            "with include_unconfigured = false, every entry is a configured Existing provider"
        );
        let ids = provider_ids(&response);
        assert!(
            ids.contains(&"custom-a") && ids.contains(&"custom-b"),
            "inserted custom providers are listed as configured"
        );
        let defaults: Vec<&str> = response
            .data
            .iter()
            .filter(|p| p.is_default)
            .map(|p| p.id.as_str())
            .collect();
        assert_eq!(
            defaults,
            vec!["custom-a"],
            "exactly the provider matching config.model_provider_id is the default"
        );
    }

    #[tokio::test]
    async fn list_providers_excludes_presets_when_not_requested() {
        let config = empty_config().await;

        let response = list_providers(
            &config,
            InterpreterProviderListParams {
                include_unconfigured: Some(false),
            },
        );

        // Quick-add presets (openrouter, anthropic, and the OpenAI ChatGPT split)
        // are not configured providers, so they must be dropped here.
        let ids = provider_ids(&response);
        assert!(
            !ids.contains(&"openrouter"),
            "preset providers must be excluded when include_unconfigured = false"
        );
        assert!(
            !ids.contains(&"anthropic"),
            "unconfigured catalog presets must be excluded"
        );
        assert!(
            !ids.contains(&"openai::chatgpt"),
            "the OpenAI ChatGPT quick-add split must be excluded"
        );
    }

    #[tokio::test]
    async fn list_providers_includes_presets_and_mirrors_tui_choices() {
        let mut config = empty_config().await;
        // Configure a provider that also exists in the bundled catalog.
        config.model_providers.insert(
            "anthropic".to_string(),
            provider("Anthropic", WireApi::Messages),
        );

        let response = list_providers(
            &config,
            InterpreterProviderListParams {
                include_unconfigured: Some(true),
            },
        );

        // A preset-only provider appears as an unconfigured QuickAdd entry, with a
        // readiness label and a non-empty description (the picker subtitle).
        let openrouter = response
            .data
            .iter()
            .find(|p| p.id == "openrouter")
            .expect("preset provider should be listed");
        assert!(!openrouter.configured);
        assert!(matches!(openrouter.kind, InterpreterProviderKind::QuickAdd));
        assert_eq!(openrouter.readiness, ProviderReadinessDto::NeedsSetup);
        assert!(!openrouter.description.is_empty());
        assert!(openrouter.starts_new_chat);

        // The configured catalog provider is not duplicated by the preset pass.
        let anthropic: Vec<&InterpreterProvider> = response
            .data
            .iter()
            .filter(|p| p.id == "anthropic")
            .collect();
        assert_eq!(
            anthropic.len(),
            1,
            "a configured provider must not be duplicated by a preset"
        );
        assert!(anthropic[0].configured);
        assert!(matches!(
            anthropic[0].kind,
            InterpreterProviderKind::Existing
        ));
        // Connection details are carried for configured providers.
        assert_eq!(anthropic[0].wire_api, Some(WireApiDto::Messages));
    }

    #[tokio::test]
    async fn list_providers_splits_openai_chatgpt_and_api_key() {
        let config = empty_config().await;

        let response = list_providers(
            &config,
            InterpreterProviderListParams {
                include_unconfigured: Some(true),
            },
        );

        let ids = provider_ids(&response);
        // The built-in `openai` provider is rendered as the ChatGPT quick-add split
        // rather than a bare `openai` entry, alongside the API-key preset.
        assert!(
            ids.contains(&"openai::chatgpt"),
            "OpenAI ChatGPT split should be present"
        );
        assert!(
            ids.contains(&"openai_api_key"),
            "OpenAI API-key preset should be present"
        );
        assert!(
            !ids.contains(&"openai"),
            "raw `openai` id should be replaced by the ChatGPT split"
        );
    }

    #[tokio::test]
    async fn list_harnesses_responses_offers_only_native() {
        let response = list_harnesses_for_wire(WireApi::Responses).await;
        assert_eq!(harness_ids(&response), BTreeSet::from([None]));
        assert_eq!(response.data.iter().filter(|h| h.is_recommended).count(), 1);
    }

    #[tokio::test]
    async fn list_harnesses_messages_offers_claude_code_variants() {
        let response = list_harnesses_for_wire(WireApi::Messages).await;
        assert_eq!(
            harness_ids(&response),
            BTreeSet::from([
                Some("claude-code".to_string()),
                Some("claude-code-bare".to_string()),
            ])
        );
        let recommended: Vec<Option<String>> = response
            .data
            .iter()
            .filter(|h| h.is_recommended)
            .map(|h| h.id.clone())
            .collect();
        assert_eq!(recommended, vec![Some("claude-code".to_string())]);
    }

    #[tokio::test]
    async fn list_harnesses_chat_offers_full_set_including_native() {
        let response = list_harnesses_for_wire(WireApi::Chat).await;
        assert_eq!(
            harness_ids(&response),
            BTreeSet::from([
                None,
                Some("claude-code".to_string()),
                Some("claude-code-bare".to_string()),
                Some("kimi-cli".to_string()),
                Some("qwen-code".to_string()),
                Some("deepseek-tui".to_string()),
                Some("mini-swe-agent".to_string()),
                Some("opencode".to_string()),
                Some("swe-agent".to_string()),
                Some("terminus-2".to_string()),
                Some("minimal".to_string()),
            ])
        );
        // With a neutral provider and no model, the native harness is recommended.
        let recommended: Vec<Option<String>> = response
            .data
            .iter()
            .filter(|h| h.is_recommended)
            .map(|h| h.id.clone())
            .collect();
        assert_eq!(recommended, vec![None]);
    }
}
