use codex_app_server_protocol::ConfigEdit as AppServerConfigEdit;
use codex_app_server_protocol::MergeStrategy;
use codex_config::types::ApprovalsReviewer;
use codex_features::FEATURES;
use codex_model_provider_info::ModelProviderInfo;
use codex_model_provider_info::WireApi;
use codex_protocol::config_types::Personality;
use codex_protocol::config_types::ServiceTier;
use codex_protocol::openai_models::ReasoningEffort;
use serde_json::Value as JsonValue;
use serde_json::json;

fn scoped_segments(profile: Option<&str>, tail: &[&str]) -> Vec<String> {
    let mut segments = Vec::with_capacity(tail.len() + usize::from(profile.is_some()) * 2);
    if let Some(profile) = profile {
        segments.push("profiles".to_string());
        segments.push(profile.to_string());
    }
    segments.extend(tail.iter().map(|segment| (*segment).to_string()));
    segments
}

fn key_path(segments: Vec<String>) -> String {
    segments.join(".")
}

pub(crate) fn set_path(segments: Vec<String>, value: JsonValue) -> AppServerConfigEdit {
    AppServerConfigEdit {
        key_path: key_path(segments),
        value,
        merge_strategy: MergeStrategy::Replace,
    }
}

pub(crate) fn clear_path(segments: Vec<String>) -> AppServerConfigEdit {
    AppServerConfigEdit {
        key_path: key_path(segments),
        value: JsonValue::Null,
        merge_strategy: MergeStrategy::Replace,
    }
}

pub(crate) fn model_selection_edits(
    profile: Option<&str>,
    model: Option<&str>,
    effort: Option<ReasoningEffort>,
) -> Vec<AppServerConfigEdit> {
    vec![
        match model {
            Some(model) => set_path(scoped_segments(profile, &["model"]), json!(model)),
            None => clear_path(scoped_segments(profile, &["model"])),
        },
        match effort {
            Some(effort) => set_path(
                scoped_segments(profile, &["model_reasoning_effort"]),
                json!(effort.to_string()),
            ),
            None => clear_path(scoped_segments(profile, &["model_reasoning_effort"])),
        },
    ]
}

pub(crate) fn model_provider_edit(profile: Option<&str>, provider_id: &str) -> AppServerConfigEdit {
    set_path(
        scoped_segments(profile, &["model_provider"]),
        json!(provider_id),
    )
}

pub(crate) fn provider_model_selection_edits(
    profile: Option<&str>,
    provider_id: &str,
    provider: Option<&ModelProviderInfo>,
    model: Option<&str>,
    effort: Option<ReasoningEffort>,
) -> Vec<AppServerConfigEdit> {
    let mut edits = vec![model_provider_edit(profile, provider_id)];
    match preferred_harness_for_provider(
        provider_id,
        provider.map(|entry| entry.name.as_str()),
        provider.and_then(|entry| entry.base_url.as_deref()),
        provider.map(|entry| entry.wire_api),
    ) {
        Some(harness) => edits.push(set_path(
            scoped_segments(profile, &["harness"]),
            json!(harness),
        )),
        None => edits.push(clear_path(scoped_segments(profile, &["harness"]))),
    }
    edits.extend(model_selection_edits(profile, model, effort));
    edits
}

pub(crate) fn preferred_harness_for_provider(
    provider_id: &str,
    provider_name: Option<&str>,
    base_url: Option<&str>,
    wire_api: Option<WireApi>,
) -> Option<&'static str> {
    if is_kimi_provider(provider_id, provider_name, base_url) {
        return Some("kimi-cli");
    }

    if matches!(wire_api, Some(WireApi::Messages)) {
        return Some("claude-code");
    }

    let provider_id = provider_id.to_ascii_lowercase();
    let provider_name = provider_name.unwrap_or_default().to_ascii_lowercase();
    let base_url = base_url.unwrap_or_default().to_ascii_lowercase();
    if provider_id.contains("anthropic")
        || provider_name.contains("anthropic")
        || base_url.contains("api.anthropic.com")
    {
        Some("claude-code")
    } else {
        None
    }
}

fn is_kimi_provider(
    provider_id: &str,
    provider_name: Option<&str>,
    base_url: Option<&str>,
) -> bool {
    let provider_id = provider_id.to_ascii_lowercase();
    let provider_name = provider_name.unwrap_or_default().to_ascii_lowercase();
    let base_url = base_url.unwrap_or_default().to_ascii_lowercase();

    provider_id.contains("kimi")
        || provider_id.contains("moonshot")
        || provider_name.contains("kimi")
        || provider_name.contains("moonshot")
        || base_url.contains("api.kimi.com")
        || base_url.contains("api.moonshot.ai")
        || base_url.contains("api.moonshot.cn")
}

pub(crate) fn service_tier_edit(
    profile: Option<&str>,
    service_tier: Option<ServiceTier>,
) -> AppServerConfigEdit {
    match service_tier {
        Some(service_tier) => set_path(
            scoped_segments(profile, &["service_tier"]),
            json!(service_tier.to_string()),
        ),
        None => clear_path(scoped_segments(profile, &["service_tier"])),
    }
}

pub(crate) fn personality_edit(
    profile: Option<&str>,
    personality: Option<Personality>,
) -> AppServerConfigEdit {
    match personality {
        Some(personality) => set_path(
            scoped_segments(profile, &["personality"]),
            json!(personality.to_string()),
        ),
        None => clear_path(scoped_segments(profile, &["personality"])),
    }
}

pub(crate) fn approvals_reviewer_edit(
    profile: Option<&str>,
    reviewer: ApprovalsReviewer,
) -> AppServerConfigEdit {
    set_path(
        scoped_segments(profile, &["approvals_reviewer"]),
        json!(reviewer.to_string()),
    )
}

pub(crate) fn plan_mode_reasoning_effort_edit(
    profile: Option<&str>,
    effort: Option<ReasoningEffort>,
) -> AppServerConfigEdit {
    match effort {
        Some(effort) => set_path(
            scoped_segments(profile, &["plan_mode_reasoning_effort"]),
            json!(effort.to_string()),
        ),
        None => clear_path(scoped_segments(profile, &["plan_mode_reasoning_effort"])),
    }
}

pub(crate) fn feature_enabled_edit(
    profile: Option<&str>,
    key: &str,
    enabled: bool,
) -> AppServerConfigEdit {
    let segments = scoped_segments(profile, &["features", key]);
    let is_default_false_feature = FEATURES
        .iter()
        .find(|spec| spec.key == key)
        .is_some_and(|spec| !spec.default_enabled);

    if enabled || profile.is_some() || !is_default_false_feature {
        set_path(segments, json!(enabled))
    } else {
        clear_path(segments)
    }
}

pub(crate) fn windows_sandbox_mode_edits(
    profile: Option<&str>,
    mode: &str,
) -> Vec<AppServerConfigEdit> {
    let mut edits = vec![set_path(
        scoped_segments(profile, &["windows", "sandbox"]),
        json!(mode),
    )];

    for key in [
        "experimental_windows_sandbox",
        "elevated_windows_sandbox",
        "enable_experimental_windows_sandbox",
    ] {
        edits.push(clear_path(scoped_segments(profile, &["features", key])));
    }

    edits
}

pub(crate) fn realtime_microphone_edit(microphone: Option<&str>) -> AppServerConfigEdit {
    match microphone {
        Some(microphone) => set_path(
            vec!["audio".to_string(), "microphone".to_string()],
            json!(microphone),
        ),
        None => clear_path(vec!["audio".to_string(), "microphone".to_string()]),
    }
}

pub(crate) fn realtime_speaker_edit(speaker: Option<&str>) -> AppServerConfigEdit {
    match speaker {
        Some(speaker) => set_path(
            vec!["audio".to_string(), "speaker".to_string()],
            json!(speaker),
        ),
        None => clear_path(vec!["audio".to_string(), "speaker".to_string()]),
    }
}

pub(crate) fn notice_flag_edit(flag: &str, acknowledged: bool) -> AppServerConfigEdit {
    set_path(
        vec!["notice".to_string(), flag.to_string()],
        json!(acknowledged),
    )
}

pub(crate) fn model_migration_seen_edit(from: &str, to: &str) -> AppServerConfigEdit {
    set_path(
        vec![
            "notice".to_string(),
            "model_migrations".to_string(),
            from.to_string(),
        ],
        json!(to),
    )
}

pub(crate) fn status_line_items_edit(items: &[String]) -> AppServerConfigEdit {
    set_path(
        vec!["tui".to_string(), "status_line".to_string()],
        json!(items),
    )
}

pub(crate) fn terminal_title_items_edit(items: &[String]) -> AppServerConfigEdit {
    set_path(
        vec!["tui".to_string(), "terminal_title".to_string()],
        json!(items),
    )
}

pub(crate) fn syntax_theme_edit(name: &str) -> AppServerConfigEdit {
    set_path(vec!["tui".to_string(), "theme".to_string()], json!(name))
}

pub(crate) fn app_enabled_edits(id: &str, enabled: bool) -> Vec<AppServerConfigEdit> {
    let enabled_segments = vec!["apps".to_string(), id.to_string(), "enabled".to_string()];
    let disabled_reason_segments = vec![
        "apps".to_string(),
        id.to_string(),
        "disabled_reason".to_string(),
    ];

    if enabled {
        vec![
            clear_path(enabled_segments),
            clear_path(disabled_reason_segments),
        ]
    } else {
        vec![
            set_path(enabled_segments, json!(false)),
            set_path(disabled_reason_segments, json!("user")),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn provider_model_selection_sets_claude_code_harness_for_anthropic() {
        let edits = provider_model_selection_edits(
            Some("work"),
            "anthropic",
            None,
            Some("claude-sonnet"),
            None,
        );

        assert_eq!(
            edits[1],
            set_path(
                vec![
                    "profiles".to_string(),
                    "work".to_string(),
                    "harness".to_string(),
                ],
                json!("claude-code"),
            )
        );
    }

    #[test]
    fn provider_model_selection_clears_harness_for_non_anthropic_provider() {
        let edits =
            provider_model_selection_edits(Some("work"), "openai", None, Some("gpt-5"), None);

        assert_eq!(
            edits[1],
            clear_path(vec![
                "profiles".to_string(),
                "work".to_string(),
                "harness".to_string(),
            ])
        );
    }

    #[test]
    fn provider_model_selection_sets_claude_code_harness_for_anthropic_base_url() {
        let provider = ModelProviderInfo {
            name: "Acme Gateway".to_string(),
            base_url: Some("https://api.anthropic.com".to_string()),
            env_key: Some("ACME_API_KEY".to_string()),
            env_key_instructions: None,
            experimental_bearer_token: None,
            auth: None,
            wire_api: codex_model_provider_info::WireApi::Responses,
            query_params: None,
            http_headers: None,
            env_http_headers: None,
            request_max_retries: None,
            stream_max_retries: None,
            stream_idle_timeout_ms: None,
            websocket_connect_timeout_ms: None,
            requires_openai_auth: false,
            supports_websockets: false,
        };
        let edits = provider_model_selection_edits(
            None,
            "compatible_acme",
            Some(&provider),
            Some("claude-sonnet"),
            None,
        );

        assert_eq!(
            edits[1],
            set_path(vec!["harness".to_string()], json!("claude-code"))
        );
    }

    #[test]
    fn provider_model_selection_sets_claude_code_harness_for_messages_wire() {
        let provider = ModelProviderInfo {
            name: "Acme Messages".to_string(),
            base_url: Some("https://gateway.example.com/messages".to_string()),
            env_key: Some("ACME_API_KEY".to_string()),
            env_key_instructions: None,
            experimental_bearer_token: None,
            auth: None,
            wire_api: WireApi::Messages,
            query_params: None,
            http_headers: None,
            env_http_headers: None,
            request_max_retries: None,
            stream_max_retries: None,
            stream_idle_timeout_ms: None,
            websocket_connect_timeout_ms: None,
            requires_openai_auth: false,
            supports_websockets: false,
        };
        let edits = provider_model_selection_edits(
            None,
            "compatible_messages",
            Some(&provider),
            Some("claude-sonnet"),
            None,
        );

        assert_eq!(
            edits[1],
            set_path(vec!["harness".to_string()], json!("claude-code"))
        );
    }

    #[test]
    fn provider_model_selection_sets_kimi_cli_harness_for_kimi_coding() {
        let provider = ModelProviderInfo {
            name: "Kimi Code".to_string(),
            base_url: Some("https://api.kimi.com/coding/v1".to_string()),
            env_key: None,
            env_key_instructions: None,
            experimental_bearer_token: None,
            auth: None,
            wire_api: WireApi::Chat,
            query_params: None,
            http_headers: None,
            env_http_headers: None,
            request_max_retries: None,
            stream_max_retries: None,
            stream_idle_timeout_ms: None,
            websocket_connect_timeout_ms: None,
            requires_openai_auth: false,
            supports_websockets: false,
        };
        let edits = provider_model_selection_edits(
            None,
            "kimi-for-coding",
            Some(&provider),
            Some("k2p5"),
            None,
        );

        assert_eq!(
            edits[1],
            set_path(vec!["harness".to_string()], json!("kimi-cli"))
        );
    }

    #[test]
    fn provider_model_selection_sets_kimi_cli_harness_for_moonshot() {
        let provider = ModelProviderInfo {
            name: "Moonshot AI".to_string(),
            base_url: Some("https://api.moonshot.ai/v1".to_string()),
            env_key: Some("MOONSHOT_API_KEY".to_string()),
            env_key_instructions: None,
            experimental_bearer_token: None,
            auth: None,
            wire_api: WireApi::Chat,
            query_params: None,
            http_headers: None,
            env_http_headers: None,
            request_max_retries: None,
            stream_max_retries: None,
            stream_idle_timeout_ms: None,
            websocket_connect_timeout_ms: None,
            requires_openai_auth: false,
            supports_websockets: false,
        };
        let edits = provider_model_selection_edits(
            None,
            "moonshotai",
            Some(&provider),
            Some("kimi-k2.5"),
            None,
        );

        assert_eq!(
            edits[1],
            set_path(vec!["harness".to_string()], json!("kimi-cli"))
        );
    }
}
