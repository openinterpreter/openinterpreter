use super::model_runtime_selectors::wait_for_model_available;
use codex_config::types::Personality;
use codex_core::TurnInputRequest;
use codex_features::Feature;
use codex_protocol::config_types::CollaborationMode;
use codex_protocol::config_types::ModeKind;
use codex_protocol::config_types::Settings;
use codex_protocol::models::BaseInstructionsProvenance;
use codex_protocol::models::PermissionProfile;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::ThreadSettingsOverrides;
use codex_protocol::user_input::UserInput;
use core_test_support::responses::mount_sse_once;
use core_test_support::responses::sse_completed;
use core_test_support::responses::start_mock_server;
use core_test_support::skip_if_no_network;
use core_test_support::test_codex::TestCodex;
use core_test_support::test_codex::local_requests;
use core_test_support::test_codex::test_codex;
use core_test_support::test_codex::turn_permission_fields;
use core_test_support::wait_for_event;
use pretty_assertions::assert_eq;
use test_case::test_case;
use wiremock::BodyPrintLimit;
use wiremock::MockServer;

const BUNDLED_FRIENDLY_TEMPLATE: &str = "You have a vivid inner life as Codex:";
const CUSTOM_INSTRUCTIONS: &str = "Custom instructions\n# Personality\nThis must remain\n## Writing Style\nThis must also remain\n# General\nGeneral instructions";

fn read_only_text_turn(
    test: &TestCodex,
    text: &str,
    model: String,
    approval_policy: AskForApproval,
) -> TurnInputRequest {
    read_only_text_turn_with_personality(test, text, model, approval_policy, None)
}

fn read_only_text_turn_with_personality(
    test: &TestCodex,
    text: &str,
    model: String,
    approval_policy: AskForApproval,
    personality: Option<Personality>,
) -> TurnInputRequest {
    let (sandbox_policy, permission_profile) =
        turn_permission_fields(PermissionProfile::read_only(), test.cwd_path());
    TurnInputRequest::user_input(vec![UserInput::Text {
        text: text.into(),
        text_elements: Vec::new(),
    }])
    .with_thread_settings(ThreadSettingsOverrides {
        environments: Some(local_requests(test.config.cwd.clone())),
        approval_policy: Some(approval_policy),
        sandbox_policy: Some(sandbox_policy),
        permission_profile,
        collaboration_mode: Some(CollaborationMode {
            mode: ModeKind::Default,
            settings: Settings {
                model,
                reasoning_effort: test.config.model_reasoning_effort.clone(),
                developer_instructions: None,
            },
        }),
        ..Default::default()
    })
}

#[test_case(None, None; "default personality")]
#[test_case(Some("none"), None; "none without feature config")]
#[test_case(Some("none"), Some(false); "none with removed feature disabled")]
#[test_case(Some("none"), Some(true); "none with removed feature enabled")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn config_personality_none_sends_no_personality() -> anyhow::Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let resp_mock = mount_sse_once(&server, sse_completed("resp-1")).await;
    let mut builder = test_codex().with_model("gpt-5.5").with_config(|config| {
        config.personality = Some(Personality::None);
    });
    let test = builder.build(&server).await?;

    test.codex
        .start_or_steer_turn(read_only_text_turn(
            &test,
            "hello",
            test.session_configured.model.clone(),
            test.config.permissions.approval_policy.value(),
        ))
        .await?;

    wait_for_event(&test.codex, |ev| matches!(ev, EventMsg::TurnComplete(_))).await;

    let request = resp_mock.single_request();
    let instructions_text = request.instructions_text();
    assert!(
        !instructions_text.contains(BUNDLED_FRIENDLY_TEMPLATE),
        "expected no friendly personality template, got: {instructions_text:?}"
    );
    let developer_texts = request.message_input_texts("developer");
    assert!(
        !developer_texts
            .iter()
            .any(|text| text.contains("<personality_spec>")),
        "did not expect a personality update message when personality is None"
    );

    Ok(())
}

#[test_case(None; "without feature config")]
#[test_case(Some(false); "with removed feature disabled")]
#[test_case(Some(true); "with removed feature enabled")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn config_personality_none_strips_baked_personality_section(
    legacy_feature_setting: Option<bool>,
) -> anyhow::Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let resp_mock = mount_sse_once(&server, sse_completed("resp-1")).await;
    let mut builder = test_codex()
        .with_model_info_override("gpt-5.5", |model_info| {
            if let Some(model_messages) = model_info.model_messages.as_mut() {
                model_messages.instructions_template = Some("Base instructions\n# Personality\nBaked personality\n## Writing Style\nNested writing style\n# General\nGeneral instructions".to_string());
            }
        })
        .with_pre_build_hook(move |home| {
            let mut config = "personality = \"none\"\n".to_string();
            if let Some(value) = legacy_feature_setting {
                config.push_str(&format!("[features]\npersonality = {value}\n"));
            }
            std::fs::write(home.join("config.toml"), config).expect("write personality config");
        });
    let test = builder.build_with_auto_env(&server).await?;

    test.codex
        .start_or_steer_turn(read_only_text_turn(
            &test,
            "hello",
            test.session_configured.model.clone(),
            test.config.permissions.approval_policy.value(),
        ))
        .await?;

    wait_for_event(&test.codex, |ev| matches!(ev, EventMsg::TurnComplete(_))).await;

    assert_eq!(
        resp_mock.single_request().instructions_text(),
        "Base instructions\n# General\nGeneral instructions"
    );

    Ok(())
}

#[test_case(CUSTOM_INSTRUCTIONS, true; "custom instructions")]
#[test_case("", false; "bridge barebones")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn config_personality_none_preserves_explicit_base_instructions(
    custom_instructions: &'static str,
    legacy_feature_setting: bool,
) -> anyhow::Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let resp_mock = mount_sse_once(&server, sse_completed("resp-1")).await;
    let mut builder = test_codex()
        .with_model("gpt-5.5")
        .with_pre_build_hook(move |home| {
            let config = format!(
                "personality = \"none\"\n[features]\npersonality = {legacy_feature_setting}\n"
            );
            std::fs::write(home.join("config.toml"), config).expect("write personality config");
        })
        .with_config(move |config| {
            config.base_instructions = Some(custom_instructions.to_string());
        });
    let test = builder.build_with_auto_env(&server).await?;

    test.codex
        .start_or_steer_turn(read_only_text_turn(
            &test,
            "hello",
            test.session_configured.model.clone(),
            test.config.permissions.approval_policy.value(),
        ))
        .await?;

    wait_for_event(&test.codex, |ev| matches!(ev, EventMsg::TurnComplete(_))).await;

    let request = resp_mock.single_request();
    let body = request.body_json();
    // Responses requests omit the instructions field for an explicit empty override.
    let expected_instructions = (!custom_instructions.is_empty())
        .then(|| serde_json::Value::String(custom_instructions.to_string()));
    assert_eq!(body.get("instructions"), expected_instructions.as_ref());
    assert!(!request.body_contains_text(BUNDLED_FRIENDLY_TEMPLATE));

    Ok(())
}

#[test_case("gpt-5.4"; "gpt_5_4")]
#[test_case("gpt-5.5"; "gpt_5_5")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fixed_instructions_ignore_legacy_personality_update(model: &str) -> anyhow::Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let responses = mount_sse_sequence(
        &server,
        vec![sse_completed("resp-1"), sse_completed("resp-2")],
    )
    .await;
    let mut builder = test_codex().with_model(model).with_config(|config| {
        config.personality = Some(Personality::Friendly);
    });
    let test = builder.build_with_auto_env(&server).await?;
    test.submit_turn("first turn").await?;

    core_test_support::submit_thread_settings(
        &test.codex,
        ThreadSettingsOverrides {
            personality: Some(Personality::Pragmatic),
            ..Default::default()
        },
    )
    .await?;
    test.submit_turn("continue with the legacy pragmatic setting")
        .await?;

    let requests = responses.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[1].instructions_text(),
        requests[0].instructions_text()
    );
    assert!(
        requests
            .iter()
            .all(|request| !request.body_contains_text("<personality_spec>"))
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remote_model_template_is_sent_literally_with_personality_feature() -> anyhow::Result<()> {
    skip_if_no_network!(Ok(()));

    let server = MockServer::builder()
        .body_print_limit(BodyPrintLimit::Limited(80_000))
        .start()
        .await;

    let remote_slug = "codex-remote-default-personality";
    let default_personality_message = "Default from remote template";
    let friendly_personality_message = "Friendly variant";
    let instructions_template = "Base instructions\n{{ personality }}\n";
    let remote_model = ModelInfo {
        slug: remote_slug.to_string(),
        display_name: "Remote default personality test".to_string(),
        description: Some("Remote model with default personality template".to_string()),
        default_reasoning_level: Some(ReasoningEffort::Medium),
        supported_reasoning_levels: vec![ReasoningEffortPreset {
            effort: ReasoningEffort::Medium,
            description: ReasoningEffort::Medium.to_string(),
        }],
        reasoning_control: Default::default(),
        supports_reasoning_summaries: false,
        shell_type: ConfigShellToolType::UnifiedExec,
        visibility: ModelVisibility::List,
        supported_in_api: true,
        priority: 1,
        additional_speed_tiers: Vec::new(),
        service_tiers: Vec::new(),
        default_service_tier: None,
        available_access_programs: None,
        upgrade: None,
        model_messages: Some(ModelMessages {
            persistent_instructions: None,
            tools: None,
            instructions_template: Some(instructions_template.to_string()),
            instructions_variables: Some(ModelInstructionsVariables {
                personality_default: Some(default_personality_message.to_string()),
                personality_friendly: Some(friendly_personality_message.to_string()),
                personality_pragmatic: Some("Pragmatic variant".to_string()),
            }),
            approvals: None,
            collaboration_modes: None,
            auto_review: None,
            permissions: None,
            multi_agent: None,
            token_budget: None,
            confirmation_policies: None,
            guardian_v2: None,
        }),
        include_skills_usage_instructions: false,
        include_plugin_usage_instructions: false,
        include_apps_usage_instructions: false,
        supports_reasoning_summary_parameter: true,
        default_reasoning_summary: ReasoningSummary::Auto,
        support_verbosity: false,
        default_verbosity: None,
        availability_nux: None,
        apply_patch_tool_type: None,
        web_search_tool_type: Default::default(),
        truncation_policy: TruncationPolicyConfig::bytes(/*limit*/ 10_000),
        supports_image_detail_original: false,
        context_window: Some(128_000),
        max_context_window: None,
        auto_compact_token_limit: None,
        comp_hash: None,
        effective_context_window_percent: 95,
        experimental_supported_tools: Vec::new(),
        input_modalities: default_input_modalities(),
        used_fallback_model_metadata: false,
        supports_search_tool: false,
        supports_experimental_context: false,
        use_responses_lite: false,
        supports_reasoning_effort_updates: false,
        guardian: None,
        node_repl_auto_review_required: false,
        node_repl_disabled: false,
        auto_review_model_override: None,
        model_specialty: None,
        tool_mode: None,
        multi_agent_version: None,
        multi_agent_reasoning_effort: None,
    };

    let _models_mock = mount_models_once(
        &server,
        ModelsResponse {
            models: vec![remote_model],
        },
    )
    .await;

    let resp_mock = mount_sse_once(&server, sse_completed("resp-1")).await;
    let mut builder = test_codex()
        .with_model_info_override("gpt-5.5", |model_info| {
            if let Some(model_messages) = model_info.model_messages.as_mut() {
                model_messages.instructions_template = Some("Base instructions\n# Personality\nBaked personality\n## Writing Style\nNested writing style\n# General\nGeneral instructions".to_string());
            }
        })
        .with_pre_build_hook(move |home| {
            let config = format!(
                "personality = \"none\"\n[features]\npersonality = {legacy_feature_setting}\n"
            );
            std::fs::write(home.join("config.toml"), config).expect("write personality config");
        })
        .with_config(move |config| {
            config.base_instructions = Some(custom_instructions.to_string());
        });
    let test = builder.build_with_auto_env(&server).await?;

    test.codex
        .start_or_steer_turn(read_only_text_turn(
            &test,
            "hello",
            test.session_configured.model.clone(),
            test.config.permissions.approval_policy.value(),
        ))
        .await?;

    wait_for_event(&test.codex, |ev| matches!(ev, EventMsg::TurnComplete(_))).await;

    let request = resp_mock.single_request();
    let instructions_text = request.instructions_text();

    assert_eq!(instructions_text, instructions_template);

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn legacy_personality_session_resumes_and_completes() -> anyhow::Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let responses = mount_sse_sequence(
        &server,
        vec![sse_completed("resp-1"), sse_completed("resp-2")],
    )
    .await;
    let legacy_instructions = "Legacy model instructions\n# Personality\nBe pragmatic.";
    let mut builder = test_codex()
        .with_model("gpt-5.5")
        .with_config(move |config| {
            config.personality = Some(Personality::Pragmatic);
            config.base_instructions = Some(legacy_instructions.to_string());
            config.base_instructions_provenance = Some(BaseInstructionsProvenance::Model {
                model: "gpt-5.5".to_string(),
            });
        });
    let original = builder.build_with_auto_env(&server).await?;
    original.submit_turn("first turn").await?;

    let mut builder = test_codex().with_model("gpt-5.5").with_config(|config| {
        config.personality = Some(Personality::None);
    });
    let resumed = builder.restart(&server, &original).await?;
    assert_eq!(
        resumed.session_configured.thread_id,
        original.session_configured.thread_id
    );
    resumed.submit_turn("continue the existing session").await?;

    let requests = responses.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].instructions_text(), legacy_instructions);
    assert!(
        requests[1]
            .message_input_texts("user")
            .iter()
            .any(|text| text == "first turn")
    );
    Ok(())
}
