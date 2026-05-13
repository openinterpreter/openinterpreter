use super::AGENTS_MD_MAX_BYTES;
use super::Config;
use super::ConfigOverrides;
use super::ConfigToml;
use super::FeatureOverrides;
use super::Features;
use super::ManagedFeatures;
use super::MultiAgentV2Config;
use super::Permissions;
use super::ProjectConfig;
use super::RealtimeAudioConfig;
use super::RealtimeConfig;
use super::UriBasedFileOpener;
use super::deserialize_config_toml_with_base;
use super::resolve_sqlite_home_env;
use super::resolve_tool_suggest_config;
use super::resolve_web_search_config;
use super::resolve_web_search_mode;
use super::thread_store_config;
use crate::agents_md::DEFAULT_AGENTS_MD_FILENAME as DEFAULT_PROJECT_DOC_FILENAME;
use crate::agents_md::LOCAL_AGENTS_MD_FILENAME as LOCAL_PROJECT_DOC_FILENAME;
use crate::config_loader::build_cli_overrides_layer;
use crate::config_loader::merge_toml_values;
use crate::path_utils::normalize_for_native_workdir;
use crate::unified_exec::DEFAULT_MAX_BACKGROUND_TERMINAL_TIMEOUT_MS;
use crate::unified_exec::MIN_EMPTY_YIELD_TIME_MS;
use codex_config::CONFIG_TOML_FILE;
use codex_config::Constrained;
use codex_config::config_toml::GhostSnapshotToml;
use codex_config::types::DEFAULT_OTEL_ENVIRONMENT;
use codex_config::types::OtelConfig;
use codex_config::types::OtelConfigToml;
use codex_config::types::OtelExporterKind;
use codex_features::Feature;
use codex_features::FeatureConfigSource;
use codex_git_utils::GhostSnapshotConfig;
use codex_model_provider_info::OPENAI_PROVIDER_ID;
use codex_model_provider_info::built_in_model_providers;
use codex_model_provider_info::default_harness_for_provider_model;
use codex_protocol::config_types::ApprovalsReviewer;
use codex_protocol::config_types::WebSearchMode;
use codex_protocol::permissions::FileSystemSandboxPolicy;
use codex_protocol::permissions::NetworkSandboxPolicy;
use codex_protocol::protocol::SandboxPolicy;
use codex_utils_absolute_path::AbsolutePathBuf;
use std::collections::BTreeMap;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use toml::Value as TomlValue;

pub(super) fn load_fast_tui_startup_config_toml(
    codex_home: &Path,
    cli_overrides: &[(String, TomlValue)],
) -> io::Result<ConfigToml> {
    let user_file = codex_home.join(CONFIG_TOML_FILE);
    let mut merged = if user_file.is_file() {
        let contents = std::fs::read_to_string(&user_file)?;
        toml::from_str::<TomlValue>(&contents)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?
    } else {
        TomlValue::Table(toml::map::Map::new())
    };

    if !cli_overrides.is_empty() {
        let cli_layer = build_cli_overrides_layer(cli_overrides);
        merge_toml_values(&mut merged, &cli_layer);
    }

    deserialize_config_toml_with_base(merged, codex_home)
}

pub(super) fn build_startup_display_config(
    cfg: ConfigToml,
    overrides: ConfigOverrides,
    codex_home: AbsolutePathBuf,
) -> io::Result<Config> {
    let ConfigOverrides {
        model,
        review_model,
        cwd,
        approval_policy,
        approvals_reviewer,
        sandbox_mode,
        model_provider,
        service_tier,
        config_profile,
        codex_self_exe,
        codex_linux_sandbox_exe,
        main_execve_wrapper_exe,
        zsh_path,
        base_instructions,
        developer_instructions,
        personality,
        compact_prompt,
        include_apply_patch_tool,
        show_raw_agent_reasoning,
        tools_web_search_request: override_tools_web_search_request,
        ephemeral,
        additional_writable_roots: _,
        permission_profile: _,
    } = overrides;

    let active_profile_name = config_profile.as_ref().or(cfg.profile.as_ref()).cloned();
    let config_profile = active_profile_name
        .as_ref()
        .and_then(|key| cfg.profiles.get(key))
        .cloned()
        .unwrap_or_default();

    let resolved_cwd = AbsolutePathBuf::try_from(normalize_for_native_workdir({
        use std::env;

        match cwd {
            None => env::current_dir()?,
            Some(path) if path.is_absolute() => path,
            Some(path) => env::current_dir()?.join(path),
        }
    }))?;

    let feature_overrides = FeatureOverrides {
        include_apply_patch_tool,
        web_search_request: override_tools_web_search_request,
    };
    let configured_features = Features::from_sources(
        FeatureConfigSource {
            features: cfg.features.as_ref(),
            include_apply_patch_tool: None,
            experimental_use_freeform_apply_patch: cfg.experimental_use_freeform_apply_patch,
            experimental_use_unified_exec_tool: cfg.experimental_use_unified_exec_tool,
        },
        FeatureConfigSource {
            features: config_profile.features.as_ref(),
            include_apply_patch_tool: config_profile.include_apply_patch_tool,
            experimental_use_freeform_apply_patch: config_profile
                .experimental_use_freeform_apply_patch,
            experimental_use_unified_exec_tool: config_profile.experimental_use_unified_exec_tool,
        },
        feature_overrides,
    );
    let features = ManagedFeatures::from_configured(configured_features, None)?;
    let active_project = cfg
        .get_active_project(resolved_cwd.as_path(), None)
        .unwrap_or(ProjectConfig { trust_level: None });

    let openai_base_url = cfg
        .openai_base_url
        .clone()
        .filter(|value| !value.is_empty());
    let mut model_providers = built_in_model_providers(openai_base_url);
    for (key, provider) in cfg.model_providers.clone() {
        model_providers.entry(key).or_insert(provider);
    }

    let model_provider_id = model_provider
        .or(config_profile.model_provider.clone())
        .or(cfg.model_provider.clone())
        .unwrap_or_else(|| OPENAI_PROVIDER_ID.to_string());
    let model_provider = model_providers
        .get(&model_provider_id)
        .cloned()
        .or_else(|| model_providers.get(OPENAI_PROVIDER_ID).cloned())
        .ok_or_else(|| io::Error::other("startup display provider registry is empty"))?;

    let model = model.or(config_profile.model.clone()).or(cfg.model.clone());
    let sandbox_policy = sandbox_mode
        .or(config_profile.sandbox_mode)
        .or(cfg.sandbox_mode)
        .map_or_else(SandboxPolicy::new_read_only_policy, |mode| match mode {
            codex_protocol::config_types::SandboxMode::ReadOnly => {
                SandboxPolicy::new_read_only_policy()
            }
            codex_protocol::config_types::SandboxMode::WorkspaceWrite => {
                SandboxPolicy::new_workspace_write_policy()
            }
            codex_protocol::config_types::SandboxMode::DangerFullAccess => {
                SandboxPolicy::DangerFullAccess
            }
        });

    let background_terminal_max_timeout = cfg
        .background_terminal_max_timeout
        .unwrap_or(DEFAULT_MAX_BACKGROUND_TERMINAL_TIMEOUT_MS)
        .max(MIN_EMPTY_YIELD_TIME_MS);

    let ghost_snapshot = startup_ghost_snapshot_config(cfg.ghost_snapshot.as_ref());
    let tui = cfg.tui.clone().unwrap_or(codex_config::types::Tui {
        notification_settings: Default::default(),
        animations: true,
        show_tooltips: true,
        alternate_screen: Default::default(),
        status_line: None,
        terminal_title: None,
        theme: None,
        model_availability_nux: Default::default(),
    });
    let otel_toml = cfg.otel.clone().unwrap_or_default();
    let log_dir = match cfg.log_dir.clone() {
        Some(log_dir) => log_dir,
        None => AbsolutePathBuf::resolve_path_against_base("log", &codex_home),
    };
    let sqlite_home = match cfg.sqlite_home.clone() {
        Some(sqlite_home) => sqlite_home,
        None => resolve_sqlite_home_env(resolved_cwd.as_path())
            .map(AbsolutePathBuf::try_from)
            .transpose()?
            .unwrap_or(codex_home.clone()),
    };
    let harness = config_profile
        .harness
        .clone()
        .or(cfg.harness.clone())
        .or_else(|| {
            default_harness_for_provider_model(
                &model_provider_id,
                &model_provider,
                model.as_deref(),
            )
            .map(ToOwned::to_owned)
        });

    Ok(Config {
        config_layer_stack: Default::default(),
        startup_warnings: Vec::new(),
        model,
        service_tier: service_tier
            .unwrap_or_else(|| config_profile.service_tier.or(cfg.service_tier)),
        review_model: review_model.or(cfg.review_model.clone()),
        model_context_window: cfg.model_context_window,
        model_auto_compact_token_limit: cfg.model_auto_compact_token_limit,
        model_provider_id,
        model_provider,
        harness,
        harness_guidance: crate::harness::guidance::DEFAULT_HARNESS_GUIDANCE_ENABLED,
        personality: personality.or(cfg.personality),
        permissions: Permissions {
            approval_policy: Constrained::allow_any(
                approval_policy
                    .or(config_profile.approval_policy)
                    .or(cfg.approval_policy)
                    .unwrap_or_default(),
            ),
            sandbox_policy: Constrained::allow_any(sandbox_policy.clone()),
            file_system_sandbox_policy: FileSystemSandboxPolicy::from_legacy_sandbox_policy(
                &sandbox_policy,
            ),
            network_sandbox_policy: NetworkSandboxPolicy::from(&sandbox_policy),
            network: None,
            allow_login_shell: cfg.allow_login_shell.unwrap_or(true),
            shell_environment_policy: cfg.shell_environment_policy.clone().into(),
            windows_sandbox_mode: cfg.windows.as_ref().and_then(|windows| windows.sandbox),
            windows_sandbox_private_desktop: cfg
                .windows
                .as_ref()
                .and_then(|windows| windows.sandbox_private_desktop)
                .unwrap_or(true),
        },
        approvals_reviewer: approvals_reviewer
            .or(config_profile.approvals_reviewer)
            .or(cfg.approvals_reviewer)
            .unwrap_or(ApprovalsReviewer::User),
        enforce_residency: Constrained::allow_any(None),
        hide_agent_reasoning: cfg.hide_agent_reasoning.unwrap_or(false),
        show_raw_agent_reasoning: cfg
            .show_raw_agent_reasoning
            .or(show_raw_agent_reasoning)
            .unwrap_or(false),
        user_instructions: None,
        base_instructions,
        developer_instructions: developer_instructions.or(cfg.developer_instructions.clone()),
        guardian_policy_config: None,
        include_permissions_instructions: config_profile
            .include_permissions_instructions
            .or(cfg.include_permissions_instructions)
            .unwrap_or(false),
        include_apps_instructions: config_profile
            .include_apps_instructions
            .or(cfg.include_apps_instructions)
            .unwrap_or(false),
        include_skill_instructions: cfg
            .skills
            .as_ref()
            .and_then(|skills| skills.include_instructions)
            .unwrap_or(true),
        include_environment_context: config_profile
            .include_environment_context
            .or(cfg.include_environment_context)
            .unwrap_or(false),
        compact_prompt: compact_prompt.or(cfg.compact_prompt.clone()),
        commit_attribution: cfg.commit_attribution.clone(),
        notify: cfg.notify.clone(),
        tui_notifications: tui.notification_settings,
        animations: tui.animations,
        show_tooltips: tui.show_tooltips,
        model_availability_nux: tui.model_availability_nux.clone(),
        tui_alternate_screen: tui.alternate_screen,
        tui_status_line: tui.status_line.clone(),
        tui_terminal_title: tui.terminal_title.clone(),
        tui_theme: tui.theme,
        cwd: resolved_cwd,
        cli_auth_credentials_store_mode: cfg.cli_auth_credentials_store.unwrap_or_default(),
        mcp_servers: Constrained::allow_any(cfg.mcp_servers.clone()),
        mcp_oauth_credentials_store_mode: cfg.mcp_oauth_credentials_store.unwrap_or_default(),
        mcp_oauth_callback_port: cfg.mcp_oauth_callback_port,
        mcp_oauth_callback_url: cfg.mcp_oauth_callback_url.clone(),
        model_providers,
        project_doc_max_bytes: cfg.project_doc_max_bytes.unwrap_or(AGENTS_MD_MAX_BYTES),
        project_doc_fallback_filenames: cfg.project_doc_fallback_filenames.clone().unwrap_or_else(
            || {
                vec![
                    DEFAULT_PROJECT_DOC_FILENAME.to_string(),
                    LOCAL_PROJECT_DOC_FILENAME.to_string(),
                ]
            },
        ),
        tool_output_token_limit: cfg.tool_output_token_limit,
        agent_max_threads: cfg.agents.as_ref().and_then(|agents| agents.max_threads),
        agent_job_max_runtime_seconds: cfg
            .agents
            .as_ref()
            .and_then(|agents| agents.job_max_runtime_seconds),
        agent_interrupt_message_enabled: cfg
            .agents
            .as_ref()
            .and_then(|agents| agents.interrupt_message)
            .unwrap_or(true),
        agent_max_depth: cfg
            .agents
            .as_ref()
            .and_then(|agents| agents.max_depth)
            .unwrap_or(3),
        agent_roles: BTreeMap::new(),
        memories: cfg.memories.clone().unwrap_or_default().into(),
        codex_home,
        sqlite_home: sqlite_home.into_path_buf(),
        log_dir: log_dir.into_path_buf(),
        history: cfg.history.clone().unwrap_or_default(),
        ephemeral: ephemeral.unwrap_or_default(),
        file_opener: cfg.file_opener.unwrap_or(UriBasedFileOpener::VsCode),
        codex_self_exe,
        codex_linux_sandbox_exe,
        main_execve_wrapper_exe,
        zsh_path: zsh_path.or_else(|| cfg.zsh_path.clone().map(PathBuf::from)),
        model_reasoning_effort: config_profile
            .model_reasoning_effort
            .or(cfg.model_reasoning_effort),
        plan_mode_reasoning_effort: config_profile
            .plan_mode_reasoning_effort
            .or(cfg.plan_mode_reasoning_effort),
        model_reasoning_summary: config_profile
            .model_reasoning_summary
            .or(cfg.model_reasoning_summary),
        model_supports_reasoning_summaries: cfg.model_supports_reasoning_summaries,
        model_catalog: None,
        model_verbosity: config_profile.model_verbosity.or(cfg.model_verbosity),
        chatgpt_base_url: config_profile
            .chatgpt_base_url
            .clone()
            .or(cfg.chatgpt_base_url.clone())
            .unwrap_or_else(|| "https://chatgpt.com/backend-api/".to_string()),
        realtime_audio: cfg
            .audio
            .clone()
            .map_or_else(RealtimeAudioConfig::default, |audio| RealtimeAudioConfig {
                microphone: audio.microphone,
                speaker: audio.speaker,
            }),
        experimental_realtime_ws_base_url: cfg.experimental_realtime_ws_base_url.clone(),
        experimental_realtime_ws_model: cfg.experimental_realtime_ws_model.clone(),
        realtime: cfg
            .realtime
            .clone()
            .map_or_else(RealtimeConfig::default, |realtime| {
                let defaults = RealtimeConfig::default();
                RealtimeConfig {
                    version: realtime.version.unwrap_or(defaults.version),
                    session_type: realtime.session_type.unwrap_or(defaults.session_type),
                    transport: realtime.transport.unwrap_or(defaults.transport),
                    voice: realtime.voice,
                }
            }),
        experimental_realtime_ws_backend_prompt: cfg
            .experimental_realtime_ws_backend_prompt
            .clone(),
        experimental_realtime_ws_startup_context: cfg
            .experimental_realtime_ws_startup_context
            .clone(),
        experimental_realtime_start_instructions: cfg
            .experimental_realtime_start_instructions
            .clone(),
        experimental_thread_config_endpoint: cfg.experimental_thread_config_endpoint.clone(),
        experimental_thread_store: thread_store_config(
            cfg.experimental_thread_store.clone(),
            cfg.experimental_thread_store_endpoint.clone(),
        ),
        forced_chatgpt_workspace_id: cfg.forced_chatgpt_workspace_id.clone(),
        forced_login_method: cfg.forced_login_method,
        include_apply_patch_tool: features.enabled(Feature::ApplyPatchFreeform),
        web_search_mode: Constrained::allow_any(
            resolve_web_search_mode(&cfg, &config_profile, &features)
                .unwrap_or(WebSearchMode::Cached),
        ),
        web_search_config: resolve_web_search_config(&cfg, &config_profile),
        use_experimental_unified_exec_tool: features.enabled(Feature::UnifiedExec),
        background_terminal_max_timeout,
        ghost_snapshot,
        multi_agent_v2: MultiAgentV2Config::default(),
        features,
        suppress_unstable_features_warning: cfg.suppress_unstable_features_warning.unwrap_or(false),
        active_profile: active_profile_name,
        active_project,
        windows_wsl_setup_acknowledged: cfg.windows_wsl_setup_acknowledged.unwrap_or(false),
        notices: cfg.notice.clone().unwrap_or_default(),
        check_for_update_on_startup: cfg.check_for_update_on_startup.unwrap_or(true),
        disable_paste_burst: cfg.disable_paste_burst.unwrap_or(false),
        analytics_enabled: config_profile
            .analytics
            .as_ref()
            .and_then(|analytics| analytics.enabled)
            .or(cfg
                .analytics
                .as_ref()
                .and_then(|analytics| analytics.enabled)),
        feedback_enabled: cfg
            .feedback
            .as_ref()
            .and_then(|feedback| feedback.enabled)
            .unwrap_or(true),
        tool_suggest: resolve_tool_suggest_config(&cfg),
        otel: startup_otel_config(otel_toml),
    })
}

fn startup_ghost_snapshot_config(toml: Option<&GhostSnapshotToml>) -> GhostSnapshotConfig {
    let mut config = GhostSnapshotConfig::default();
    if let Some(toml) = toml
        && let Some(ignore_over_bytes) = toml.ignore_large_untracked_files
    {
        config.ignore_large_untracked_files = if ignore_over_bytes > 0 {
            Some(ignore_over_bytes)
        } else {
            None
        };
    }
    if let Some(toml) = toml
        && let Some(threshold) = toml.ignore_large_untracked_dirs
    {
        config.ignore_large_untracked_dirs = if threshold > 0 { Some(threshold) } else { None };
    }
    if let Some(toml) = toml
        && let Some(disable_warnings) = toml.disable_warnings
    {
        config.disable_warnings = disable_warnings;
    }
    config
}

fn startup_otel_config(toml: OtelConfigToml) -> OtelConfig {
    let log_user_prompt = toml.log_user_prompt.unwrap_or(false);
    let environment = toml
        .environment
        .unwrap_or(DEFAULT_OTEL_ENVIRONMENT.to_string());
    let exporter = toml.exporter.unwrap_or(OtelExporterKind::None);
    let trace_exporter = toml.trace_exporter.unwrap_or_else(|| exporter.clone());
    let metrics_exporter = toml.metrics_exporter.unwrap_or(OtelExporterKind::Statsig);
    OtelConfig {
        log_user_prompt,
        environment,
        exporter,
        trace_exporter,
        metrics_exporter,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ConfigProfile;
    use crate::config::ProjectConfig;
    use crate::config::TrustLevel;
    use codex_protocol::config_types::AltScreenMode;
    use pretty_assertions::assert_eq;
    use std::collections::HashMap;
    use tempfile::TempDir;

    #[test]
    fn fast_startup_config_toml_reads_user_config_and_cli_overrides() {
        let codex_home = TempDir::new().expect("create temp dir");
        std::fs::write(
            codex_home.path().join(CONFIG_TOML_FILE),
            r#"
model = "gpt-5"
model_provider = "openrouter"
[profiles.fast]
model = "gpt-5-mini"
"#,
        )
        .expect("write config");

        let config = load_fast_tui_startup_config_toml(
            codex_home.path(),
            &[(
                String::from("profile"),
                TomlValue::String(String::from("fast")),
            )],
        )
        .expect("load fast startup config");

        assert_eq!(config.profile, Some(String::from("fast")));
        assert_eq!(config.model, Some(String::from("gpt-5")));
        assert_eq!(config.model_provider, Some(String::from("openrouter")));
    }

    #[test]
    fn startup_display_config_uses_profile_model_and_tui_settings() {
        let mut cfg = ConfigToml {
            model: Some(String::from("gpt-5")),
            ..Default::default()
        };
        cfg.profile = Some(String::from("fast"));
        cfg.tui = Some(codex_config::types::Tui {
            animations: false,
            alternate_screen: AltScreenMode::Never,
            theme: Some(String::from("nord")),
            ..Default::default()
        });
        cfg.profiles.insert(
            String::from("fast"),
            ConfigProfile {
                model: Some(String::from("gpt-5-mini")),
                ..Default::default()
            },
        );

        let codex_home = TempDir::new().expect("create temp dir");
        let config = build_startup_display_config(
            cfg,
            ConfigOverrides::default(),
            codex_home.path().to_path_buf(),
        )
        .expect("build startup display config");

        assert_eq!(config.model, Some(String::from("gpt-5-mini")));
        assert_eq!(config.tui_alternate_screen, AltScreenMode::Never);
        assert_eq!(config.tui_theme, Some(String::from("nord")));
        assert!(!config.animations);
    }

    #[test]
    fn startup_display_config_preserves_active_project_trust() {
        let cfg = ConfigToml {
            projects: Some(HashMap::from([(
                String::from("/tmp/project"),
                ProjectConfig {
                    trust_level: Some(TrustLevel::Trusted),
                },
            )])),
            ..Default::default()
        };
        let overrides = ConfigOverrides {
            cwd: Some(PathBuf::from("/tmp/project")),
            ..Default::default()
        };

        let codex_home = TempDir::new().expect("create temp dir");
        let config = build_startup_display_config(cfg, overrides, codex_home.path().to_path_buf())
            .expect("build startup display config");

        assert_eq!(
            config.active_project,
            ProjectConfig {
                trust_level: Some(TrustLevel::Trusted),
            }
        );
    }
}
