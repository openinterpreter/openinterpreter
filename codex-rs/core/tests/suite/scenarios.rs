//! Multi-turn Astra scenarios snapshot the model-visible request history of shipped features.

use std::collections::HashMap;
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use codex_config::ConfigLayerSource;
use codex_config::ConfigLayerStack;
use codex_config::types::McpServerConfig;
use codex_context_fragments::AnsweredQuestion;
use codex_context_fragments::ContextualUserFragment;
use codex_core::StartThreadOptions;
use codex_core::TurnInput;
use codex_core::TurnInputRequest;
use codex_core::config::Config;
use codex_exec_server::EnvironmentManager;
use codex_exec_server::LOCAL_ENVIRONMENT_ID;
use codex_extension_api::ExtensionDataInit;
use codex_extension_api::ExtensionRegistry;
use codex_extension_api::ExtensionRegistryBuilder;
use codex_features::Feature;
use codex_login::CodexAuth;
use codex_models_manager::bundled_models_response;
use codex_protocol::capabilities::CapabilityRootLocation;
use codex_protocol::capabilities::SelectedCapabilityRoot;
use codex_protocol::dynamic_tools::DynamicToolCallOutputContentItem;
use codex_protocol::dynamic_tools::DynamicToolFunctionSpec;
use codex_protocol::dynamic_tools::DynamicToolResponse;
use codex_protocol::dynamic_tools::DynamicToolSpec;
use codex_protocol::items::AgentMessageDelivery;
use codex_protocol::items::TurnItem;
use codex_protocol::models::ImageReference;
use codex_protocol::models::PermissionProfile;
use codex_protocol::models::PermissionProfileSnapshot;
use codex_protocol::models::ResponseItem;
use codex_protocol::openai_models::ToolMessage;
use codex_protocol::openai_models::ToolMode;
use codex_protocol::permissions::NetworkSandboxPolicy;
use codex_protocol::protocol::AdditionalContextEntry;
use codex_protocol::protocol::AdditionalContextKind;
use codex_protocol::protocol::EnvironmentConfigState;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::Op;
use codex_protocol::protocol::ThreadSettingsOverrides;
use codex_protocol::protocol::TurnEnvironmentSelection;
use codex_protocol::protocol::TurnEnvironmentSelections;
use codex_protocol::protocol::TurnSettingsUpdate;
use codex_protocol::protocol::TurnSettingsUpdateOutcome;
use codex_protocol::request_user_input::RequestUserInputAnswer;
use codex_protocol::request_user_input::RequestUserInputResponse;
use codex_protocol::user_input::UserInput;
use codex_skills_extension::ExecutorSkillProvider;
use codex_skills_extension::SkillProviders;
use codex_skills_extension::SkillsExtensionConfig;
use codex_skills_extension::install;
use codex_skills_extension::install_with_providers;
use codex_utils_path_uri::PathUri;
use core_test_support::context_snapshot;
use core_test_support::context_snapshot::ContextSnapshotOptions;
use core_test_support::context_snapshot::SnapshotEntry;
use core_test_support::exec_server::ExecServerProcess;
use core_test_support::responses::ev_assistant_message;
use core_test_support::responses::ev_completed;
use core_test_support::responses::ev_custom_tool_call;
use core_test_support::responses::ev_function_call_with_namespace;
use core_test_support::responses::ev_message_item_added;
use core_test_support::responses::ev_output_text_delta;
use core_test_support::responses::ev_response_created;
use core_test_support::responses::mount_sse_sequence;
use core_test_support::responses::sse;
use core_test_support::responses::start_mock_server;
use core_test_support::skip_if_no_network;
use core_test_support::skip_if_wine_exec;
use core_test_support::stdio_server_bin;
use core_test_support::streaming_sse::StreamingSseChunk;
use core_test_support::streaming_sse::start_streaming_sse_server;
use core_test_support::test_codex::TestCodex;
use core_test_support::test_codex::TestCodexBuilder;
use core_test_support::test_codex::TestEnv;
use core_test_support::test_codex::environment_config_for_selection;
use core_test_support::test_codex::executor_path_uri;
use core_test_support::test_codex::test_codex;
use core_test_support::wait_for_event;
use core_test_support::wait_for_event_match;
use core_test_support::wait_for_mcp_server;
use serde_json::json;
use tempfile::TempDir;
use tokio::sync::oneshot;

const ONE_PIXEL_PNG_BASE64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==";

fn compaction_output(value: &str) -> serde_json::Value {
    json!({"type": "response.output_item.done", "item": {
        "type": "compaction", "encrypted_content": value
    }})
}

#[path = "scenarios_agent_message_board.rs"]
mod agent_message_board;

#[path = "scenarios_audio_file_tests.rs"]
mod audio_file;

#[path = "scenarios/catalog_settings.rs"]
mod catalog_settings;

#[path = "scenarios_catalog_approval_instructions_tests.rs"]
mod catalog_approval_instructions;

#[path = "scenarios_agent_eviction_tests.rs"]
mod agent_eviction;

#[path = "scenarios_budget_context.rs"]
mod budget_context;

#[path = "scenarios_catalog_apps_instructions_tests.rs"]
mod catalog_apps_instructions;

#[path = "scenarios_context_management_tools.rs"]
mod context_management_tools;

#[path = "scenarios_compaction_tests.rs"]
mod compaction;

#[path = "scenarios_content_filter.rs"]
mod content_filter;

#[path = "scenarios_credential_masking.rs"]
mod credential_masking;

#[path = "scenarios_code_mode_settled_helpers_tests.rs"]
mod code_mode_settled_helpers;

#[path = "scenarios_command_sessions_tests.rs"]
mod command_sessions;

#[path = "scenarios_dynamic_tools.rs"]
mod dynamic_tools;

#[path = "scenarios_exec_tools.rs"]
mod exec_tools;

#[path = "scenarios_frozen_cloud_skills.rs"]
mod frozen_cloud_skills;

#[path = "scenarios_guardian_sender_context.rs"]
mod guardian_sender_context;

#[path = "scenarios_guardian_async_elastic.rs"]
mod guardian_async_elastic;

#[path = "scenarios_guardian_heartbeat.rs"]
mod guardian_heartbeat;

#[path = "scenarios_guardian_agent_messages_tests.rs"]
mod guardian_agent_messages;

#[path = "scenarios_guardian_conversation_history_tests.rs"]
mod guardian_conversation_history;

#[path = "scenarios_guardian_direct_review.rs"]
mod guardian_direct_review;

#[path = "scenarios_guardian_direct_mcp.rs"]
mod guardian_direct_mcp;

#[path = "scenarios_guardian_extra_policy.rs"]
mod guardian_extra_policy;

#[path = "scenarios_guardian_handoff.rs"]
mod guardian_handoff;

#[path = "scenarios_guardian_orchestrator_connector.rs"]
mod guardian_orchestrator_connector;

#[path = "scenarios_guardian_reconsideration.rs"]
mod guardian_reconsideration;

#[path = "host_turn_admission.rs"]
mod host_turn_admission;

#[path = "scenarios_injected_agent_mail.rs"]
mod injected_agent_mail;

#[path = "scenarios_indirect_namespace_prefixes.rs"]
mod indirect_namespace_prefixes;

#[path = "scenarios_mailbox_preemption_tests.rs"]
mod mailbox_preemption;

#[path = "scenarios_multi_environment_mcp.rs"]
mod multi_environment_mcp;

#[path = "scenarios_managed_human_approval_tests.rs"]
mod managed_human_approval;

#[path = "scenarios_mcp_ema.rs"]
mod mcp_ema;

#[path = "scenarios_mcp_resource_messages.rs"]
mod mcp_resource_messages;

#[path = "scenarios_mcp_sharing.rs"]
mod mcp_sharing;

#[path = "scenarios_mxc.rs"]
mod mxc;

#[path = "scenarios_incremental_tools_resume.rs"]
mod incremental_tools_resume;

#[path = "scenarios_incremental_tools.rs"]
mod incremental_tools;

#[path = "scenarios_model_instructions.rs"]
mod model_instructions;

#[path = "scenarios_provider_capabilities.rs"]
mod provider_capabilities;

#[path = "scenarios_partial_answers.rs"]
mod partial_answers;

#[path = "scenarios_preparation.rs"]
mod preparation;

#[path = "scenarios_retained_tool_outputs.rs"]
mod retained_tool_outputs;

#[path = "scenarios_reserved_spawn.rs"]
mod reserved_spawn;

#[path = "scenarios_shared_instructions.rs"]
mod shared_instructions;

#[path = "scenarios_skill_catalog_dedup.rs"]
mod skill_catalog_dedup;

#[path = "scenarios_strict_3p_cache.rs"]
mod strict_3p_cache;

#[path = "scenarios_tools_namespace_budget.rs"]
mod tools_namespace_budget;

#[path = "scenarios_web_search.rs"]
mod web_search;

#[path = "scenarios_world_state_prefix_tests.rs"]
mod world_state_prefix;

#[path = "scenarios_turn_input_agent_message.rs"]
mod turn_input_agent_message;

fn skills_extensions() -> Arc<ExtensionRegistry<Config>> {
    let mut extensions = ExtensionRegistryBuilder::<Config>::new();
    install(&mut extensions, |config: &Config| SkillsExtensionConfig {
        include_instructions: config.include_skill_instructions,
        max_context_tokens: config.skill_max_context_tokens,
        bundled_skills_enabled: config.bundled_skills_enabled(),
        orchestrator_skills_enabled: config.orchestrator_skills_enabled,
        shadow_selection_enabled: config.features.enabled(Feature::SkillSearch),
    });
    Arc::new(extensions.build())
}

fn write_skill(path: &Path, name: &str, description: &str, body: &str) -> Result<PathBuf> {
    fs::create_dir_all(path)?;
    let skill = path.join("SKILL.md");
    fs::write(
        &skill,
        format!("---\nname: {name}\ndescription: {description}\n---\n\n{body}\n"),
    )?;
    Ok(fs::canonicalize(skill)?)
}

struct ScenarioSkills {
    outline: PathBuf,
    agenda: PathBuf,
    summarize: PathBuf,
    final_check: PathBuf,
}

fn write_scenario_capabilities(home: &TempDir) -> Result<ScenarioSkills> {
    fs::write(
        home.path().join("config.toml"),
        "[features]\nplugins = true\n\n[skills.bundled]\nenabled = false\n\n[plugins.\"calendar@test\"]\nenabled = true\n\n[plugins.\"notes@test\"]\nenabled = true\n",
    )?;
    let plugin_cache = home.path().join("plugins/cache/test");
    for (name, description) in [
        ("calendar", "Prepare a team schedule"),
        ("notes", "Summarize meeting notes"),
    ] {
        let manifest = plugin_cache
            .join(name)
            .join("local/.codex-plugin/plugin.json");
        fs::create_dir_all(manifest.parent().expect("manifest parent"))?;
        fs::write(
            manifest,
            json!({ "name": name, "description": description }).to_string(),
        )?;
    }

    Ok(ScenarioSkills {
        outline: write_skill(
            &home.path().join("skills/outline"),
            "outline",
            "Draft a project outline",
            "List the goals and owners.",
        )?,
        agenda: write_skill(
            &plugin_cache.join("calendar/local/skills/agenda"),
            "agenda",
            "Plan a team agenda",
            "List meetings with dates and attendees.",
        )?,
        summarize: write_skill(
            &plugin_cache.join("notes/local/skills/summarize"),
            "summarize",
            "Summarize team notes",
            "Extract decisions and action items.",
        )?,
        final_check: write_skill(
            &home.path().join("skills/final-check"),
            "final-check",
            "Review a final brief",
            "Check that the brief has an owner for every action.",
        )?,
    })
}

fn text(value: &str) -> UserInput {
    UserInput::Text {
        text: value.to_string(),
        text_elements: Vec::new(),
    }
}

fn selected_skill(name: &str, path: &Path) -> UserInput {
    UserInput::Skill {
        name: name.to_string(),
        path: path.to_path_buf(),
    }
}

fn plugin(name: &str) -> UserInput {
    UserInput::Mention {
        name: name.to_string(),
        path: format!("plugin://{name}@test"),
    }
}

fn configure_scenario_catalog(config: &mut Config) {
    // Keep the fixture independent of the checkout's project configuration.
    let stack = &config.config_layer_stack;
    config.config_layer_stack = ConfigLayerStack::new(
        stack
            .all_layers_low_to_high()
            .filter(|layer| !matches!(&layer.name, ConfigLayerSource::Project { .. }))
            .cloned()
            .collect(),
        stack.requirements().clone(),
        stack.requirements_toml().clone(),
    )
    .expect("fixture config layers");
    config.model_catalog = Some(bundled_models_response().expect("bundled model catalog"));
    config.orchestrator_skills_enabled = false;
}

fn configure_catalog_plugin_scenario(config: &mut Config) {
    configure_scenario_catalog(config);
    config.workspace_roots = vec![config.cwd.clone()];
    let stack = &config.config_layer_stack;
    let fixture_config = stack.effective_config();
    // Keep the fixture's plugin and App policy while avoiding discovery of the real
    // $HOME/.agents/skills, which a User layer adds alongside the fixture's skills.
    config.config_layer_stack = ConfigLayerStack::new(
        vec![ConfigLayerEntry::new(
            ConfigLayerSource::System {
                file: config.codex_home.join("config.toml"),
            },
            fixture_config,
        )],
        stack.requirements().clone(),
        stack.requirements_toml().clone(),
    )
    .expect("isolated plugin scenario config")
    .with_cloud_config_binding(stack.cloud_config_binding().cloned());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gif_generation_preserves_animation_and_replays_a_still_preview() -> Result<()> {
    skip_if_no_network!(Ok(()));
    // Two one-pixel frames, red then blue, with a 200 ms frame duration.
    let gif = "R0lGODlhAQABAIEAAP8AAAAAAAAAAAAAACH/C05FVFNDQVBFMi4wAwEAAAAh+QQAFAAAACwAAAAAAQABAAAIBAABBAQAIfkEARQAAQAsAAAAAAEAAQCBAAD/AAAAAAAAAAAACAQAAQQEADs=";
    let server = start_mock_server().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/v1/images/generations"))
        .and(wiremock::matchers::body_json(json!({
            "prompt": "A pixel alternating between red and blue.",
            "background": "opaque",
            "model": "gpt-image-2",
            "quality": "auto",
            "size": "auto",
            "output_format": "gif",
            "frame_count": 2,
            "frame_duration_ms": 200,
        })))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
            "created": 1,
            "data": [{"b64_json": gif}],
        })))
        .expect(1)
        .mount(&server)
        .await;

    let auth = CodexAuth::create_dummy_chatgpt_auth_for_testing();
    let mut extensions = ExtensionRegistryBuilder::<Config>::new();
    install_image_generation_extension(
        &mut extensions,
        codex_core::test_support::auth_manager_from_auth(auth.clone()),
        |_config| None,
    );
    let image_store = Arc::new(RecordingFileAttachmentStore::default());
    let test = test_codex()
        .with_auth(auth)
        .with_extensions(Arc::new(extensions.build()))
        .with_image_store(image_store.clone())
        .with_config(|config| {
            configure_scenario_catalog(config);
            config
                .features
                .enable(Feature::ImageGenerationAnimation)
                .unwrap();
        })
        .with_model_info_override("gpt-5.4", |model| {
            model.use_responses_lite = true;
            model.input_modalities = vec![InputModality::Text, InputModality::Image];
        })
        .build_with_auto_env(&server)
        .await?;
    let mock = mount_sse_sequence(
        &server,
        vec![
            sse(vec![
                ev_function_call_with_namespace(
                    "gif-generation",
                    "image_gen",
                    "imagegen",
                    &json!({
                        "prompt": "A pixel alternating between red and blue.",
                        "gif": {"frame_count": 2, "frame_duration_ms": 200},
                    })
                    .to_string(),
                ),
                ev_completed("generated"),
            ]),
            sse(vec![
                ev_assistant_message("preview", "Here is the two-frame GIF."),
                ev_completed("previewed"),
            ]),
            sse(vec![
                ev_assistant_message("timing", "Each frame lasts 200 ms."),
                ev_completed("answered"),
            ]),
        ],
    )
    .await;
    test.submit_text_turn("Generate a two-frame GIF of a pixel alternating red and blue.")
        .await?;
    test.submit_text_turn("How long does each frame last?")
        .await?;

    let uploads = image_store.uploads.lock().expect("uploads lock");
    let [animation, preview] = uploads.as_slice() else {
        panic!("expected an animated artifact and a separate model preview");
    };
    assert_eq!(animation.data, BASE64_STANDARD.decode(gif)?);
    assert_eq!(image::guess_format(&preview.data)?, image::ImageFormat::Png);
    assert_eq!(
        image::load_from_memory(&preview.data)?
            .to_rgba8()
            .get_pixel(0, 0)
            .0,
        [255, 0, 0, 255]
    );
    let requests = mock.requests();
    assert_eq!(requests.len(), 3);
    insta::assert_snapshot!(context_snapshot::format_request_history_snapshot(
        "GIF generation exposes animation controls and guidance, preserves the animated artifact, and replays a still preview through a follow-up turn.",
        &requests,
        &ContextSnapshotOptions::default()
            .rewrite_known_segments()
            .include_request_settings(),
    ));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn admitted_prompt_id_reaches_responses() -> Result<()> {
    skip_if_no_network!(Ok(()));
    let server = start_mock_server().await;
    let mut builder = test_codex().with_config(|config| {
        configure_scenario_catalog(config);
        config.include_environment_context = false;
    });
    let test = builder.build_with_auto_env(&server).await?;
    let mock = mount_sse_sequence(
        &server,
        vec![
            sse(vec![ev_completed("first")]),
            sse(vec![ev_completed("second")]),
        ],
    )
    .await;
    let uuid = uuid::Uuid::new_v4();
    for id in [Some(uuid), None] {
        let mut request = TurnInputRequest::user_input(vec![text("Repeat this input.")]);
        request.user_item_uuid = id;
        test.codex.start_or_steer_turn(request).await?;
        wait_for_event(&test.codex, |event| {
            matches!(event, EventMsg::TurnComplete(_))
        })
        .await;
    }

    let requests = mock.requests();
    assert_eq!(requests.len(), 2);
    let reserved = codex_protocol::ResponseItemId::for_admitted_user_input(uuid);
    for (index, request) in requests.iter().enumerate() {
        let input = request.input();
        let prompts: Vec<_> = input
            .iter()
            .filter(|item| {
                item["role"] == "user" && item["content"][0]["text"] == "Repeat this input."
            })
            .collect();
        assert_eq!(prompts.len(), index + 1);
        assert_eq!(prompts[0]["id"], reserved.as_str());
        if let Some(legacy) = prompts.get(1) {
            let id = legacy["id"].as_str().expect("legacy message id");
            assert!(id.starts_with("msg_") && !id.starts_with("msg_input_"));
        }
    }
    insta::assert_snapshot!(context_snapshot::format_request_history_snapshot(
        "An admitted prompt and an ordinary prompt with identical text reach Responses on successive turns.",
        &requests,
        &ContextSnapshotOptions::default().rewrite_known_segments(),
    ));
    Ok(())
}

#[test_case::test_case(false; "messages")]
#[test_case::test_case(true; "tool_outputs")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn additional_context_tool_outputs(tool_outputs: bool) -> Result<()> {
    skip_if_no_network!(Ok(()));
    let server = start_mock_server().await;
    let test = test_codex()
        .with_config(move |config| {
            configure_scenario_catalog(config);
            config.include_environment_context = false;
            if !tool_outputs {
                config
                    .features
                    .disable(Feature::UntrustedAsToolOutput)
                    .expect("test config should allow feature update");
            }
        })
        .build_with_auto_env(&server)
        .await?;
    let mock = mount_sse_sequence(
        &server,
        (0..4)
            .map(|index| sse(vec![ev_completed(&format!("response-{index}"))]))
            .collect(),
    )
    .await;

    for (prompt, name, value) in [
        (
            "Summarize the selected document.",
            Some("browser"),
            "The launch is on Monday.",
        ),
        (
            "The same document is now open in the editor.",
            Some("editor"),
            "The launch is on Monday.",
        ),
        (
            "Check the date again.",
            Some("editor"),
            "The launch is on Monday.",
        ),
        (
            "Summarize the updated document.",
            None,
            "The launch is on Tuesday.",
        ),
    ] {
        test.codex
            .start_or_steer_turn(
                TurnInputRequest::user_input(vec![text(prompt)]).with_additional_context(
                    std::collections::BTreeMap::from([
                        (
                            "selection".to_owned(),
                            AdditionalContextEntry {
                                value: value.to_owned(),
                                kind: AdditionalContextKind::Untrusted,
                                name: name.map(str::to_owned),
                            },
                        ),
                        (
                            "application".to_owned(),
                            AdditionalContextEntry::new(
                                "Keep the answer brief.".to_owned(),
                                AdditionalContextKind::Application,
                            ),
                        ),
                    ]),
                ),
            )
            .await?;
        wait_for_event(&test.codex, |event| {
            matches!(event, EventMsg::TurnComplete(_))
        })
        .await;
    }
    let requests = mock.requests();
    assert_eq!(requests.len(), 4);
    if tool_outputs {
        assert_eq!(
            requests
                .iter()
                .map(|request| request.inputs_of_type("function_call_output").len())
                .collect::<Vec<_>>(),
            vec![1, 2, 2, 3]
        );
        assert_eq!(requests[3].message_input_texts("user").len(), 4);
        let input = requests[0].input();
        let context_index = input
            .iter()
            .position(|item| item["type"] == "function_call_output")
            .expect("context output");
        let user_index = input
            .iter()
            .position(|item| item["role"] == "user")
            .expect("user message");
        assert!(context_index < user_index);
    } else {
        assert!(
            requests
                .iter()
                .all(|request| request.inputs_of_type("function_call_output").is_empty())
        );
        assert_eq!(
            requests
                .iter()
                .map(|request| request.message_input_texts("user").len())
                .collect::<Vec<_>>(),
            vec![2, 3, 4, 6]
        );
    }
    insta::assert_snapshot!(
        if tool_outputs {
            "additional_context_tool_outputs"
        } else {
            "additional_context_legacy_messages"
        },
        context_snapshot::format_request_history_snapshot(
            "Named context changes are appended before the user; unchanged values are deduplicated.",
            &requests,
            &ContextSnapshotOptions::default().rewrite_known_segments(),
        )
    );
    Ok(())
}

#[tokio::test]
async fn cached_stdio_stays_dormant_in_a_new_chat_until_first_use() -> Result<()> {
    skip_if_no_network!(Ok(()));
    skip_if_wine_exec!(Ok(()), "requires a Windows test_stdio_server binary");
    let requests = super::mcp_tool_cache::cached_stdio_first_use_scenario().await?;
    let entries = requests.iter().map(SnapshotEntry::body).collect::<Vec<_>>();
    insta::assert_snapshot!(context_snapshot::format_context_snapshot(
        "A new chat uses a cached MCP catalog for unrelated work, then calls echo. The live server updates the tool definitions and marks cwd as app-only.",
        &entries,
        &ContextSnapshotOptions::default()
            .rewrite_known_segments()
            .include_request_settings(),
    ));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn realtime_attachment_replacement_preserves_handoff_context() -> Result<()> {
    let requests = super::realtime_attachment::attachment_replacement_scenario().await?;
    insta::assert_snapshot!(
        "realtime_attachment_replacement",
        context_snapshot::format_request_history_snapshot(
            "A replacement fences an older pending connection and stale stops; only the selected call starts a Codex handoff.",
            &requests,
            &ContextSnapshotOptions::default().rewrite_known_segments(),
        )
    );
    Ok(())
}

/// Expiring files change request copies while canonical history retains the original references.
/// A shared barrier requires the three distinct file IDs to resolve concurrently on each request.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unavailable_images_are_rechecked_without_rewriting_history() -> Result<()> {
    skip_if_no_network!(Ok(()));
    let server = start_mock_server().await;
    let image_store = Arc::new(RecordingFileAttachmentStore {
        resolve_barrier: Some(Arc::new(tokio::sync::Barrier::new(/*n*/ 3))),
        ..Default::default()
    });
    let test = test_codex()
        .with_model("gpt-6-astra")
        .with_image_store(image_store.clone())
        .with_config(configure_scenario_catalog)
        .build_with_auto_env(&server)
        .await?;
    let mock = mount_sse_sequence(
        &server,
        vec![
            sse(vec![
                ev_assistant_message("first", "The images are available."),
                ev_completed("first"),
            ]),
            sse(vec![
                ev_assistant_message("second", "Some images are no longer available."),
                ev_completed("second"),
            ]),
        ],
    )
    .await;
    let file_ids = [
        "file_expiring",
        "file_available",
        "file_expiring",
        "file_backend_error",
    ];
    let mut input = vec![text("Compare these images.")];
    input.extend(file_ids.map(|file_id| UserInput::Image {
        image: ImageReference::File {
            file_id: file_id.to_string(),
        },
        detail: Some(ImageDetail::High),
    }));
    test.codex
        .start_or_steer_turn(TurnInputRequest::user_input(input))
        .await?;
    wait_for_event(&test.codex, |event| {
        matches!(event, EventMsg::TurnComplete(_))
    })
    .await;
    assert_eq!(
        *image_store.resolves.lock().expect("resolves lock"),
        HashMap::from([
            ("file_expiring".to_string(), 1),
            ("file_available".to_string(), 1),
            ("file_backend_error".to_string(), 1),
        ])
    );

    image_store
        .resolve_errors
        .lock()
        .expect("resolve errors lock")
        .extend([
            (
                "file_expiring".to_string(),
                AttachmentStoreErrorKind::NotFound,
            ),
            (
                "file_backend_error".to_string(),
                AttachmentStoreErrorKind::Backend,
            ),
        ]);
    test.submit_text_turn("Compare them again.").await?;
    assert_eq!(
        *image_store.resolves.lock().expect("resolves lock"),
        HashMap::from([
            ("file_expiring".to_string(), 2),
            ("file_available".to_string(), 2),
            ("file_backend_error".to_string(), 2),
        ])
    );
    let requests = mock.requests();
    assert_eq!(requests.len(), 2);
    assert!(
        requests[0]
            .body_json()
            .to_string()
            .contains("file_expiring")
    );
    let unavailable_parts = requests[1]
        .input()
        .into_iter()
        .filter_map(|item| {
            item["internal_chat_message_metadata_passthrough"]["content_item_kinds"]
                .as_array()
                .cloned()
        })
        .flatten()
        .filter(|kind| kind == "images.unavailable")
        .count();
    assert_eq!(unavailable_parts, 2);
    assert!(requests[1].body_contains_text("[image unavailable]"));
    assert!(
        !requests[1]
            .body_json()
            .to_string()
            .contains("file_expiring")
    );
    assert!(
        requests[1]
            .body_json()
            .to_string()
            .contains("file_backend_error")
    );
    assert!(
        requests[1]
            .body_json()
            .to_string()
            .contains("file_available")
    );

    let recorded_images = test
        .codex
        .load_history(/*include_archived*/ false)
        .await?
        .items
        .into_iter()
        .filter_map(|item| match item {
            RolloutItem::ResponseItem(envelope) => match envelope.item {
                ResponseItem::Message { content, .. } => Some(content),
                _ => None,
            },
            _ => None,
        })
        .flatten()
        .filter(|content| matches!(content, ContentItem::InputImage { .. }))
        .collect::<Vec<_>>();
    assert_eq!(
        recorded_images,
        file_ids.map(|file_id| ContentItem::InputImage {
            image: ImageReference::File {
                file_id: file_id.to_string()
            },
            detail: Some(ImageDetail::High),
        })
    );
    insta::assert_snapshot!(context_snapshot::format_request_history_snapshot(
        "Available file references pass through. A later request replaces missing files, preserves references on backend errors, rechecks each distinct ID once, and keeps canonical history intact.",
        &requests,
        &ContextSnapshotOptions::default().rewrite_known_segments(),
    ));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn code_mode_forwards_mixed_mcp_content() -> Result<()> {
    skip_if_no_network!(Ok(()));
    skip_if_wine_exec!(Ok(()), "requires a Windows test_stdio_server binary");
    let server = start_mock_server().await;
    let (_test, response) = super::code_mode::run_code_mode_turn_with_rmcp_model(
        &server,
        "Show the caption and image from the MCP image tool.",
        r#"const result = await tools.mcp__rmcp__image_scenario({scenario: "text_then_image", caption: "sample"});
content(result.content);"#,
        "gpt-5.5",
    )
    .await?;
    let output = response.single_request().custom_tool_call_output("call-1");
    assert_eq!(output["output"][1]["text"], "sample");
    assert_eq!(output["output"][2]["type"], "input_image");
    insta::assert_snapshot!(context_snapshot::format_request_history_snapshot(
        "Code Mode forwards MCP text and image blocks to the model in their original order.",
        &core_test_support::responses::received_responses_requests(&server).await,
        &ContextSnapshotOptions::default().rewrite_known_segments(),
    ));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn code_mode_mcp_schema_limits_preserve_explicit_overrides() -> Result<()> {
    skip_if_no_network!(Ok(()));
    let requests = super::code_mode::mcp_schema_max_bytes_scenario().await?;
    insta::assert_snapshot!(
        "code_mode_mcp_schema_limits",
        context_snapshot::format_request_history_snapshot(
            "Code Mode exposes explicit MCP schema limits in its prompt and runtime tool catalog.",
            &requests,
            &ContextSnapshotOptions::default()
                .rewrite_known_segments()
                .include_request_settings(),
        )
    );
    Ok(())
}

#[test_case::test_case(ToolMode::Direct; "direct")]
#[test_case::test_case(ToolMode::CodeModeOnly; "code_mode")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_tool_discovery_in_same_turn(tool_mode: ToolMode) -> Result<()> {
    skip_if_no_network!(Ok(()));
    skip_if_wine_exec!(Ok(()), "requires a Windows test_stdio_server binary");
    let requests = super::rmcp_client::mcp_tool_discovery_scenario(
        tool_mode, /*invalidation*/ None, /*enabled*/ true,
    )
    .await?;
    let mut bodies = requests
        .iter()
        .map(core_test_support::responses::ResponsesRequest::body_json)
        .collect::<Vec<_>>();
    // Cargo and Bazel can serialize embedded JSON in different key orders.
    // Normalize each output text before snapshot truncation and hashing.
    for body in &mut bodies {
        for item in body["input"].as_array_mut().expect("request input") {
            if item["type"] != "function_call_output" && item["type"] != "custom_tool_call_output" {
                continue;
            }
            let parts = match &mut item["output"] {
                serde_json::Value::Array(parts) => parts
                    .iter_mut()
                    .map(|part| &mut part["text"])
                    .collect::<Vec<_>>(),
                output => vec![output],
            };
            for part in parts {
                let text = part.as_str().expect("text output");
                let mut normalized = context_snapshot::normalize_json_lines(text);
                if text.ends_with('\n') {
                    normalized.push('\n');
                }
                *part = normalized.into();
            }
        }
    }
    let entries = bodies.iter().map(SnapshotEntry::body).collect::<Vec<_>>();
    insta::assert_snapshot!(
        format!("mcp_tool_discovery_{tool_mode:?}"),
        context_snapshot::format_context_snapshot(
            "An MCP discovery call announces a new tool, which is called in the same turn.",
            &entries,
            &ContextSnapshotOptions::default()
                .rewrite_known_segments()
                .include_request_settings(),
        )
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn astra_asks_an_async_question_and_receives_the_answer_while_working() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let question = "Who should receive the launch update?";
    let (release_continuation, continuation_gate) = oneshot::channel();
    let mut working_message = ev_assistant_message("working", "I drafted a short launch update.");
    working_message["item"]["phase"] = json!("commentary");
    let (streaming, _completions) = start_streaming_sse_server(vec![
        vec![StreamingSseChunk {
            gate: None,
            body: sse(vec![
                ev_response_created("question-response"),
                ev_function_call_with_namespace(
                    "audience-question",
                    "functions",
                    "request_user_input_async",
                    &json!({"questions": [{"title": question, "options": ["Internal team", "Customers"]}]}).to_string(),
                ),
                ev_completed("question-response"),
            ]),
        }],
        vec![
            StreamingSseChunk {
                gate: None,
                body: sse(vec![ev_response_created("working-response"), working_message]),
            },
            StreamingSseChunk {
                gate: Some(continuation_gate),
                body: sse(vec![ev_completed("working-response")]),
            },
        ],
        vec![StreamingSseChunk {
            gate: None,
            body: sse(vec![
                ev_response_created("answered-response"),
                ev_assistant_message("answered", "Here is the launch update for customers."),
                ev_completed("answered-response"),
            ]),
        }],
        vec![StreamingSseChunk {
            gate: None,
            body: sse(vec![
                ev_response_created("follow-up-response"),
                ev_assistant_message("follow-up", "The email subject is: Launch update."),
                ev_completed("follow-up-response"),
            ]),
        }],
    ])
    .await;
    let config_server = start_mock_server().await;
    let base_url = format!("{}/v1", streaming.uri());
    let test = test_codex()
        .with_model("gpt-6-astra")
        .with_auth(CodexAuth::create_dummy_chatgpt_auth_for_testing())
        .with_config(move |config| {
            configure_scenario_catalog(config);
            config.model_provider.base_url = Some(base_url);
            // The gated mock records raw request bodies for the shared snapshot renderer.
            config
                .features
                .disable(Feature::EnableRequestCompression)
                .expect("disable compression for the gated mock");
        })
        .build_with_auto_env(&config_server)
        .await?;

    test.codex
        .start_or_steer_turn(TurnInputRequest::user_input(vec![text(
            "Draft a short launch update. Ask me who it is for and keep working while I answer.",
        )]))
        .await?;
    let turn_id = wait_for_event_match(&test.codex, |event| match event {
        EventMsg::TurnStarted(event) => Some(event.turn_id.clone()),
        _ => None,
    })
    .await;
    wait_for_event(&test.codex, |event| {
        matches!(event, EventMsg::ItemCompleted(event)
            if matches!(&event.item, TurnItem::AgentMessage(message)
                if message.delivery == Some(AgentMessageDelivery::Async)))
    })
    .await;
    tokio::time::timeout(
        Duration::from_secs(/*secs*/ 10),
        streaming.wait_for_request_count(/*count*/ 2),
    )
    .await?;
    wait_for_event(&test.codex, |event| {
        matches!(event, EventMsg::ItemCompleted(event)
            if matches!(&event.item, TurnItem::AgentMessage(message) if message.id == "working"))
    })
    .await;

    let question_id = json!(["request_user_input_async", "audience-question", 0]).to_string();
    let answer = AnsweredQuestion::new(&question_id, question, "Customers").render();
    test.codex
        .steer_turn(TurnInputRequest::user_input(vec![text(&answer)]), turn_id)
        .await?;
    release_continuation.send(()).expect("release continuation");
    wait_for_event(&test.codex, |event| {
        matches!(event, EventMsg::TurnComplete(_))
    })
    .await;
    test.submit_turn("Now give it an email subject.").await?;

    let requests = streaming
        .requests()
        .await
        .iter()
        .map(|body| serde_json::from_slice(body))
        .collect::<serde_json::Result<Vec<_>>>()?;
    let entries = requests.iter().map(SnapshotEntry::body).collect::<Vec<_>>();
    insta::assert_snapshot!(
        "astra_async_question_and_answer",
        context_snapshot::format_context_snapshot(
            "Astra asks who a launch update is for, keeps working, and receives the user's answer in the active turn.",
            &entries,
            &ContextSnapshotOptions::default().rewrite_known_segments(),
        )
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn astra_switches_environments_for_the_rest_of_the_active_turn() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let mock = mount_sse_sequence(
        &server,
        vec![
            sse(vec![
                ev_function_call_with_namespace(
                    "pause-for-switch",
                    "functions",
                    "request_user_input",
                    &json!({"questions": [{
                        "id": "continue", "header": "Continue", "question": "Continue in the other workspace?",
                        "options": [
                            {"label": "Yes", "description": "Continue working."},
                            {"label": "No", "description": "Stop working."}
                        ]
                    }]}).to_string(),
                ),
                ev_completed("first-step"),
            ]),
            sse(vec![
                ev_assistant_message("active-turn", "I continued in the other workspace."),
                ev_completed("second-step"),
            ]),
            sse(vec![
                ev_assistant_message("next-turn", "I am back in the original workspace."),
                ev_completed("next-turn"),
            ]),
        ],
    )
    .await;
    let test = test_codex()
        .with_model("gpt-6-astra")
        .with_auth(CodexAuth::create_dummy_chatgpt_auth_for_testing())
        .with_config(|config| {
            configure_scenario_catalog(config);
            config.workspace_roots = vec![config.cwd.clone()];
            config
                .features
                .enable(Feature::DefaultModeRequestUserInput)
                .expect("enable request_user_input");
            config
                .features
                .enable(Feature::PerEnvironmentContext)
                .expect("enable per-environment context");
        })
        .build_with_auto_env(&server)
        .await?;
    let executor = ExecServerProcess::start().await?;
    let first_workspace = TempDir::new()?;
    test.thread_manager
        .environment_manager()
        .upsert_environment(
            "environment-a".to_string(),
            executor.websocket_url.clone(),
            /*connect_timeout*/ None,
        )?;
    let mut first = test.executor_environment().selection().clone();
    first.environment_id = "environment-a".to_string();
    first.cwd = PathUri::from_host_native_path(first_workspace.path())?;
    first.workspace_roots = vec![first.cwd.clone()];
    let mut first_config = environment_config_for_selection(&test.config, &first);
    first_config.permission_profile =
        PermissionProfileSnapshot::legacy(PermissionProfile::read_only());
    first.config = EnvironmentConfigState::Ready(first_config);

    let mut original = test.executor_environment().selection().clone();
    let mut original_config = environment_config_for_selection(&test.config, &original);
    original_config.permission_profile =
        PermissionProfileSnapshot::legacy(PermissionProfile::workspace_write_with(
            &[],
            NetworkSandboxPolicy::Enabled,
            /*exclude_tmpdir_env_var*/ true,
            /*exclude_slash_tmp*/ true,
        ));
    original.config = EnvironmentConfigState::Ready(original_config);
    let mut other = original.clone();
    other.cwd = original.cwd.join("other-workspace")?;
    other.workspace_roots = vec![other.cwd.clone()];
    let mut other_config = environment_config_for_selection(&test.config, &other);
    other_config.permission_profile =
        PermissionProfileSnapshot::legacy(PermissionProfile::read_only());
    other.config = EnvironmentConfigState::Ready(other_config);
    test.fs()
        .create_directory(
            &other.cwd,
            CreateDirectoryOptions {
                recursive: false,
                follow_symlinks: true,
            },
            /*sandbox*/ None,
        )
        .await?;

    test.codex
        .start_or_steer_turn(
            TurnInputRequest::user_input(vec![text(
                "Ask before continuing in the other workspace.",
            )])
            .with_thread_settings(ThreadSettingsOverrides {
                environments: Some(
                    TurnEnvironmentSelections::new(
                        test.config.cwd.clone(),
                        vec![first.clone(), original],
                    )
                    .into_requests(),
                ),
                ..Default::default()
            }),
        )
        .await?;
    let request = wait_for_event_match(&test.codex, |event| match event {
        EventMsg::RequestUserInput(request) => Some(request.clone()),
        _ => None,
    })
    .await;
    let (reply, outcome) = oneshot::channel();
    test.codex
        .submit(Op::TurnSettings {
            turn_id: request.turn_id.clone(),
            update: TurnSettingsUpdate {
                environments: Some(vec![first.into_request(), other.into_request()]),
                ..Default::default()
            },
            reply,
        })
        .await?;
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(/*secs*/ 10), outcome).await??,
        TurnSettingsUpdateOutcome::Applied
    );
    test.codex
        .submit(Op::UserInputAnswer {
            id: request.turn_id,
            response: RequestUserInputResponse {
                answers: HashMap::from([(
                    "continue".to_string(),
                    RequestUserInputAnswer {
                        answers: vec!["Yes".to_string()],
                    },
                )]),
            },
        })
        .await?;
    wait_for_event(&test.codex, |event| {
        matches!(event, EventMsg::TurnComplete(_))
    })
    .await;
    test.submit_text_turn("Which workspace are you using now?")
        .await?;

    let requests = mock.requests();
    let snapshot = context_snapshot::format_request_history_snapshot(
        "Astra sees two environments with different permissions. Only the second environment's workspace and permissions change for the next step; the original defaults return on the next turn.",
        &requests,
        &ContextSnapshotOptions::default(),
    )
    .replace(r#"<environment id="local""#, r#"<environment id="environment-b""#)
    .replace(r#"<environment id="remote""#, r#"<environment id="environment-b""#);
    insta::assert_snapshot!("astra_active_environment_selection", snapshot);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn astra_kickoff_with_skills_plugins_and_remote_compaction() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let home = Arc::new(TempDir::new()?);
    let skills = write_scenario_capabilities(&home)?;
    let mock = mount_sse_sequence(
        &server,
        vec![
            sse(vec![
                ev_assistant_message("draft", "Agenda drafted for the kickoff."),
                ev_completed("draft-response"),
            ]),
            sse(vec![
                ev_assistant_message("notes", "The team chose Friday and assigned owners."),
                ev_completed("notes-response"),
            ]),
            sse(vec![
                json!({
                    "type": "response.output_item.done",
                    "item": {
                        "type": "compaction",
                        "encrypted_content": "SCENARIO_REMOTE_CHECKPOINT",
                    }
                }),
                ev_completed("compact-response"),
            ]),
            sse(vec![
                ev_assistant_message("final", "Here is the checked kickoff brief."),
                ev_completed("final-response"),
            ]),
        ],
    )
    .await;
    let mut builder = test_codex()
        .with_model("gpt-6-astra")
        .with_home(home)
        .with_auth(CodexAuth::create_dummy_chatgpt_auth_for_testing())
        .with_extensions(skills_extensions())
        .with_workspace_setup(|cwd, fs| async move {
            fs.write_file(
                &executor_path_uri(cwd.join("AGENTS.md"))?,
                b"Kickoff updates must name an owner and a date.".to_vec(),
                Default::default(),
                /*sandbox*/ None,
            )
            .await?;
            Ok::<(), anyhow::Error>(())
        })
        .with_config(|config| {
            configure_scenario_catalog(config);
        });
    let test = builder.build(&server).await?;

    for input in [
        vec![
            text("Plan a team kickoff for Friday using $outline and $calendar:agenda."),
            selected_skill("outline", &skills.outline),
            selected_skill("calendar:agenda", &skills.agenda),
            plugin("calendar"),
        ],
        vec![
            text("Summarize the kickoff notes with $notes:summarize."),
            selected_skill("notes:summarize", &skills.summarize),
            plugin("notes"),
        ],
    ]
    .into_iter()
    {
        test.codex
            .start_or_steer_turn(TurnInputRequest::user_input(input))
            .await?;
        wait_for_event(&test.codex, |event| {
            matches!(event, EventMsg::TurnComplete(_))
        })
        .await;
    }

    test.codex.submit(Op::Compact).await?;
    wait_for_event(&test.codex, |event| {
        matches!(event, EventMsg::TurnComplete(_))
    })
    .await;

    test.codex
        .start_or_steer_turn(TurnInputRequest::user_input(vec![
            text("Check the final kickoff brief and attached sketch with $final-check and $calendar:agenda."),
            UserInput::Image {
                image: ImageReference::Inline {
                    image_url: format!("data:image/png;base64,{ONE_PIXEL_PNG_BASE64}"),
                },
                detail: None,
            },
            selected_skill("final-check", &skills.final_check),
            plugin("calendar"),
        ]))
        .await?;
    wait_for_event(&test.codex, |event| {
        matches!(event, EventMsg::TurnComplete(_))
    })
    .await;

    let requests = mock.requests();
    insta::assert_snapshot!(
        "astra_kickoff_remote_compaction_windows",
        context_snapshot::format_request_history_snapshot(
            "Astra plans a kickoff with local and plugin skills, remotely compacts, and checks an image brief.",
            &requests,
            &ContextSnapshotOptions::default().include_request_settings(),
        )
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn astra_omits_disabled_executor_skills_from_model_context() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let skill_files = TempDir::new()?;
    let skill_root = fs::canonicalize(skill_files.path())?.join("skills");
    let disabled = write_skill(
        &skill_root.join("retired-helper"),
        "retired-helper",
        "Use the retired workflow",
        "Follow the retired workflow.",
    )?;
    write_skill(
        &skill_root.join("active-helper"),
        "active-helper",
        "Use the current workflow",
        "Follow the current workflow.",
    )?;
    let provider = ExecutorSkillProvider::new_with_restriction_product(
        Arc::new(EnvironmentManager::default_for_tests()),
        /*restriction_product*/ None,
    )
    .with_disabled_skill_paths(HashMap::from([(
        LOCAL_ENVIRONMENT_ID.to_string(),
        HashSet::from([PathUri::from_host_native_path(disabled)?]),
    )]));
    let mut extensions = ExtensionRegistryBuilder::<Config>::new();
    install_with_providers(
        &mut extensions,
        SkillProviders::new().with_executor_provider(Arc::new(provider)),
        |config: &Config| SkillsExtensionConfig {
            include_instructions: config.include_skill_instructions,
            max_context_tokens: config.skill_max_context_tokens,
            bundled_skills_enabled: false,
            orchestrator_skills_enabled: false,
            shadow_selection_enabled: false,
        },
    );
    let mock = mount_sse_sequence(
        &server,
        vec![sse(vec![
            ev_assistant_message("skills", "The active-helper skill is available."),
            ev_completed("skills-response"),
        ])],
    )
    .await;
    let test = test_codex()
        .with_model("gpt-6-astra")
        .with_auth(CodexAuth::create_dummy_chatgpt_auth_for_testing())
        .with_extensions(Arc::new(extensions.build()))
        .with_config(configure_scenario_catalog)
        .build(&server)
        .await?;
    let skill_root = PathUri::from_host_native_path(skill_root)?;
    let root_locator = format!(
        "skill://workspace-skills/{}",
        skill_root
            .inferred_native_path_string()
            .replace('\\', "/")
            .trim_start_matches('/')
    );
    let mut thread_extension_init = ExtensionDataInit::new();
    thread_extension_init.insert(vec![SelectedCapabilityRoot {
        id: "workspace-skills".to_string(),
        location: CapabilityRootLocation::Environment {
            environment_id: LOCAL_ENVIRONMENT_ID.to_string(),
            path: skill_root,
        },
    }]);
    let thread = test
        .thread_manager
        .start_thread(StartThreadOptions {
            thread_extension_init,
            ..StartThreadOptions::new(test.config.clone())
        })
        .await?
        .thread;
    thread
        .start_or_steer_turn(TurnInputRequest::user_input(vec![text(
            "Which workflow skills are available?",
        )]))
        .await?;
    wait_for_event(&thread, |event| matches!(event, EventMsg::TurnComplete(_))).await;

    let requests = mock.requests();
    assert_eq!(requests.len(), 1);
    let mut body = requests[0].body_json();
    let input = body["input"].to_string();
    assert!(input.contains("active-helper"));
    assert!(!input.contains("retired-helper"));
    // Normalize opaque skill locators before snapshot truncation and hashing.
    body["input"] = serde_json::from_str(
        &input.replace(&root_locator, "skill://workspace-skills/<SKILLS_ROOT>"),
    )?;
    insta::assert_snapshot!(
        "astra_disabled_executor_skills",
        context_snapshot::format_context_snapshot(
            "Astra sees the active executor skill while the caller-disabled skill is omitted.",
            &[SnapshotEntry::body(&body)],
            &ContextSnapshotOptions::default().include_request_settings(),
        )
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn multi_agent_catalog_parameters() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let test = test_codex()
        .with_config(|config| {
            configure_scenario_catalog(config);
            config.workspace_roots = vec![config.cwd.clone()];
        })
        .with_model_info_override("gpt-6-astra", |model| {
            model.model_messages.as_mut().expect("model messages").tools = Some(
                serde_json::from_value(json!({"multi_agent": {"list_agents": {
                    "parameters": json!({
                        "type": "object",
                        "properties": {"path_prefix": {
                            "type": "string",
                            "description": "Inspect agents within this task path.",
                            "minLength": 1,
                            "maxLength": 128,
                        }},
                        "required": ["path_prefix"],
                        "additionalProperties": false,
                    }).to_string(),
                }}}))
                .expect("catalog tool messages"),
            );
        })
        .build_with_auto_env(&server)
        .await?;
    let mock = mount_sse_sequence(
        &server,
        vec![
            sse(vec![
                ev_response_created("agents-response"),
                ev_function_call_with_namespace(
                    "agents-call",
                    "collaboration",
                    "list_agents",
                    r#"{"path_prefix":"/root"}"#,
                ),
                ev_completed("agents-response"),
            ]),
            sse(vec![
                ev_assistant_message("final", "Only the root agent is working on this task."),
                ev_completed("final-response"),
            ]),
        ],
    )
    .await;
    test.submit_turn("Check which agents are working under /root before delegating more work.")
        .await?;
    insta::assert_snapshot!(
        "multi_agent_catalog_parameters",
        context_snapshot::format_request_history_snapshot(
            "Astra calls list_agents using the selected catalog parameter schema.",
            &mock.requests(),
            &ContextSnapshotOptions::default().include_request_settings(),
        )
    );
    Ok(())
}

#[cfg_attr(windows, ignore = "the fixture uses a Unix shell command")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn astra_settings_release_check_with_direct_and_code_mode_tools() -> Result<()> {
    skip_if_no_network!(Ok(()));
    skip_if_wine_exec!(Ok(()), "the fixture uses a Unix shell command");

    let server = start_mock_server().await;
    let rmcp_server_bin = stdio_server_bin()?;
    let home = Arc::new(TempDir::new()?);
    let mut builder = test_codex()
        .with_model("gpt-6-astra")
        .with_home(home)
        .with_auth(CodexAuth::create_dummy_chatgpt_auth_for_testing())
        .with_config(move |config| {
            configure_scenario_catalog(config);
            let mut servers = config.mcp_servers.get().clone();
            servers.insert(
                "rmcp".to_string(),
                serde_json::from_value::<McpServerConfig>(json!({
                    "command": rmcp_server_bin,
                    "env": { "MCP_TEST_VALUE": "release-check" },
                }))
                .expect("test MCP server config"),
            );
            config.mcp_servers.set(servers).expect("test MCP servers");
        });
    let test = builder.build(&server).await?;
    let release = test.cwd_path().join("release");
    fs::create_dir_all(&release)?;
    let diagnostics = std::iter::once("UI-42: expected Settings button label: Apply".to_string())
        .chain((1..=120).map(|line| format!("diagnostic {line:03}: rendering settings panel")))
        .chain(std::iter::once(
            "UI-42: observed Settings button label: Save".to_string(),
        ))
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(release.join("diagnostics.log"), diagnostics)?;
    fs::write(release.join("status.md"), "Status: pending\n")?;
    fs::write(
        release.join("settings.png"),
        BASE64_STANDARD.decode(ONE_PIXEL_PNG_BASE64)?,
    )?;
    wait_for_mcp_server(&test.codex, "rmcp").await?;

    let patch = "*** Begin Patch\n*** Update File: release/status.md\n@@\n-Status: pending\n+Status: blocked\n+Reason: expected Apply; observed Save\n+MCP: reachable\n*** End Patch\n";
    let patch_code = format!("text(await tools.apply_patch(`{patch}`));");
    let mock = mount_sse_sequence(
        &server,
        vec![
            sse(vec![
                ev_response_created("agents-response"),
                ev_function_call_with_namespace("agents-call", "collaboration", "list_agents", "{}"),
                ev_completed("agents-response"),
            ]),
            sse(vec![
                ev_response_created("diagnostics-response"),
                ev_custom_tool_call(
                    "diagnostics-call",
                    "exec",
                    r#"const [log, ping] = await Promise.all([
  tools.exec_command({ cmd: "cat release/diagnostics.log", login: false, max_output_tokens: 4000 }),
  tools.mcp__rmcp__echo({ message: "settings-release-check" }),
]);
text(`diagnostics:\n${log.output}`);
text(`MCP: ${ping.structuredContent?.echo ?? "missing"}`);"#,
                ),
                ev_completed("diagnostics-response"),
            ]),
            sse(vec![
                ev_response_created("image-response"),
                ev_custom_tool_call(
                    "image-call",
                    "exec",
                    "image(await tools.view_image({ path: \"release/settings.png\", detail: \"original\" }));",
                ),
                ev_completed("image-response"),
            ]),
            sse(vec![
                ev_response_created("patch-response"),
                ev_custom_tool_call("patch-call", "exec", &patch_code),
                ev_completed("patch-response"),
            ]),
            sse(vec![
                ev_response_created("readback-response"),
                ev_custom_tool_call(
                    "readback-call",
                    "exec",
                    "text((await tools.exec_command({ cmd: \"cat release/status.md\", login: false })).output);",
                ),
                ev_completed("readback-response"),
            ]),
            sse(vec![
                ev_assistant_message(
                    "final",
                    "The Settings release is blocked: expected Apply, observed Save. The MCP integration responded and no other agent is working on this task.",
                ),
                ev_completed("final-response"),
            ]),
        ],
    )
    .await;

    test.submit_turn("Check the Settings release. Read release/diagnostics.log, inspect release/settings.png, confirm the local MCP integration responds, and update release/status.md with the result. Tell me whether another agent is working on this task.").await?;
    insta::assert_snapshot!(
        "astra_settings_release_check_tool_shapes",
        context_snapshot::format_request_history_snapshot(
            "Astra checks a Settings release using direct collaboration and Code Mode tools.",
            &mock.requests(),
            &ContextSnapshotOptions::default().include_request_settings(),
        )
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn astra_reads_code_mode_call_timing() -> Result<()> {
    skip_if_no_network!(Ok(()));
    let server = start_mock_server().await;
    let test = test_codex()
        .with_model("gpt-6-astra")
        .with_config(|config| {
            configure_scenario_catalog(config);
            // Use the selected cwd as the workspace root on local and remote executors.
            config.workspace_roots = vec![config.cwd.clone()];
            config.code_mode.experimental_show_cell_overhead = true;
            config
                .features
                .enable(Feature::CodeMode)
                .expect("enable code mode");
            config
                .features
                .enable(Feature::CodeModeOnly)
                .expect("enable code-mode-only tools");
            config
                .features
                .enable(Feature::CodeModeHost)
                .expect("enable the code-mode host");
        })
        .build_with_auto_env(&server)
        .await?;
    let mock = mount_sse_sequence(
        &server,
        vec![
            sse(vec![
                ev_response_created("exec-response"),
                ev_custom_tool_call("exec-call", "exec", "text('ready');"),
                ev_completed("exec-response"),
            ]),
            sse(vec![
                ev_response_created("wait-response"),
                ev_function_call_with_namespace(
                    "wait-call",
                    "functions",
                    "wait",
                    r#"{"cell_id":"missing"}"#,
                ),
                ev_completed("wait-response"),
            ]),
            sse(vec![
                ev_assistant_message("final", "The call completed; the missing-cell wait failed."),
                ev_completed("final-response"),
            ]),
        ],
    )
    .await;
    test.submit_turn("Run a code cell, then inspect its timing and a failed wait.")
        .await?;
    insta::assert_snapshot!(
        "astra_code_mode_call_timing",
        context_snapshot::format_request_history_snapshot(
            "Astra receives host and handler timings on completed and failed code-mode calls.",
            &mock.requests(),
            &ContextSnapshotOptions::default(),
        )
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn astra_refreshes_plugin_tools_and_skills_in_an_existing_thread() -> Result<()> {
    skip_if_no_network!(Ok(()));
    core_test_support::skip_if_remote!(Ok(()), "plugin and MCP fixtures use host-local paths");

    let server = start_mock_server().await;
    let home = Arc::new(TempDir::new()?);
    let config = "[features]\nplugins = true\n\n[skills.bundled]\nenabled = false\n";
    fs::write(home.path().join("config.toml"), config)?;
    let lookup = r#"text(ALL_TOOLS.filter(({ name }) => name === "mcp__notes__echo").map(({ name }) => name));"#;
    let mock = mount_sse_sequence(
        &server,
        vec![
            sse(vec![
                ev_response_created("before-refresh"),
                ev_custom_tool_call("before-lookup", "exec", lookup),
                ev_completed("before-refresh"),
            ]),
            sse(vec![
                ev_assistant_message("missing-tool", "The Notes tool is not installed yet."),
                ev_completed("missing-tool-response"),
            ]),
            sse(vec![
                ev_response_created("after-refresh"),
                ev_custom_tool_call("after-lookup", "exec", lookup),
                ev_completed("after-refresh"),
            ]),
            sse(vec![
                ev_response_created("first-echo"),
                ev_custom_tool_call(
                    "first-echo-call",
                    "exec",
                    r#"const result = await tools.mcp__notes__echo({ message: "Mira owns the kickoff" }); text(result.structuredContent?.echo);"#,
                ),
                ev_completed("first-echo"),
            ]),
            sse(vec![
                ev_assistant_message("owner", "The Notes plugin confirms Mira owns the kickoff."),
                ev_completed("owner-response"),
            ]),
            sse(vec![
                ev_response_created("follow-up"),
                ev_custom_tool_call(
                    "second-echo-call",
                    "exec",
                    r#"const result = await tools.mcp__notes__echo({ message: "The kickoff is Friday" }); text(result.structuredContent?.echo);"#,
                ),
                ev_completed("follow-up"),
            ]),
            sse(vec![
                ev_assistant_message("deadline", "The Notes plugin confirms the kickoff is Friday."),
                ev_completed("deadline-response"),
            ]),
        ],
    )
    .await;
    let mut builder = test_codex()
        .with_model("gpt-6-astra")
        .with_home(Arc::clone(&home))
        .with_auth(CodexAuth::create_dummy_chatgpt_auth_for_testing())
        .with_extensions(skills_extensions())
        .with_config(configure_scenario_catalog);
    let test = builder.build(&server).await?;

    test.submit_turn("Check whether the Notes plugin echo tool is available.")
        .await?;

    let plugin_root = home.path().join("plugins/cache/test/notes/local");
    fs::create_dir_all(plugin_root.join(".codex-plugin"))?;
    fs::write(
        plugin_root.join(".codex-plugin/plugin.json"),
        json!({ "name": "notes", "description": "Look up and summarize team notes" }).to_string(),
    )?;
    fs::write(
        plugin_root.join(".mcp.json"),
        json!({ "mcpServers": { "notes": { "command": stdio_server_bin()?, "cwd": "." } } })
            .to_string(),
    )?;
    let skill = write_skill(
        &plugin_root.join("skills/summarize"),
        "summarize",
        "Summarize team notes",
        "State the owner and date from the notes.",
    )?;
    fs::write(
        home.path().join("config.toml"),
        format!("{config}\n[plugins.\"notes@test\"]\nenabled = true\n"),
    )?;
    test.codex.submit(Op::ReloadUserConfig).await?;
    test.codex
        .start_or_steer_turn(TurnInputRequest::user_input(vec![
            text("I installed Notes. Use $notes:summarize and the Notes tool to check that Mira owns the kickoff."),
            plugin("notes"),
            selected_skill("notes:summarize", &skill),
        ]))
        .await?;
    wait_for_event(&test.codex, |event| {
        matches!(event, EventMsg::TurnComplete(_))
    })
    .await;

    test.submit_text_turn("Use the Notes tool again to check that the kickoff is Friday.")
        .await?;

    insta::assert_snapshot!(
        "astra_plugin_refresh",
        context_snapshot::format_request_history_snapshot(
            "Astra checks for Notes, refreshes its installed plugin without restarting, and uses the new skill and Code Mode MCP tool across turns.",
            &mock.requests(),
            &ContextSnapshotOptions::default().include_request_settings(),
        )
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn subagent_browser_auth_resolves_user_prompt() -> Result<()> {
    skip_if_no_network!(Ok(()));
    skip_if_wine_exec!(Ok(()), "the MCP fixture requires a host Python interpreter");
    use super::mcp_subagent_elicitation::Caller;
    use super::mcp_subagent_elicitation::RequestKind;
    use super::mcp_subagent_elicitation::mcp_server_elicitation_scenario;

    let requests =
        mcp_server_elicitation_scenario(Caller::Subagent, RequestKind::BrowserAuth).await?;
    let snapshot = context_snapshot::format_request_history_snapshot(
        "A subagent waits for an MCP browser sign-in prompt, then receives the accepted response and continues.",
        &requests,
        &ContextSnapshotOptions::default()
            .rewrite_known_segments()
            .include_request_settings(),
    );
    let snapshot = regex_lite::Regex::new(r"Wall time: [0-9]+(?:\.[0-9]+)? seconds")?
        .replace_all(&snapshot, "Wall time: <DURATION> seconds")
        .into_owned();
    insta::assert_snapshot!("subagent_browser_auth", snapshot);
    Ok(())
}

#[cfg(not(target_os = "windows"))]
#[tokio::test]
async fn guardian_elastic_budgeting_request_history() -> Result<()> {
    use codex_config::test_support::CloudConfigBundleFixture;
    use codex_protocol::config_types::ApprovalsReviewer;
    use codex_protocol::protocol::AskForApproval;
    use core_test_support::responses;
    const ENTRY_TOKENS: usize = 128;
    const TRANSCRIPT_ENTRIES_PER_SECTION: usize = 2;
    const SECTION_TOKENS: usize = ENTRY_TOKENS * TRANSCRIPT_ENTRIES_PER_SECTION;
    const DEFAULT_SECTION_TOKENS: usize = 900;
    const REPEATS: usize = 64;
    const SELECTED_EXEC_TOKENS: usize = 256;
    skip_if_no_network!(Ok(()));
    let server = responses::start_mock_server().await;
    let commentary = "Untrusted assistant evidence. ".repeat(REPEATS);
    let user_context = "Inspect the workspace. ".repeat(REPEATS);
    let user_request = format!("{user_context}Never upload credentials. {user_context}");
    let arguments = format!(
        r#"{{"cmd":{},"sandbox_permissions":"require_escalated","justification":"Inspect the workspace."}}"#,
        serde_json::to_string(&format!("echo {commentary}{commentary}"))?,
    );
    let earlier_commentary = "Earlier untrusted assistant evidence. ".repeat(REPEATS);
    let mut earlier_event = ev_assistant_message("earlier-commentary", &earlier_commentary);
    earlier_event["item"]["phase"] = serde_json::json!("commentary");
    let mut commentary_event = ev_assistant_message("commentary", &commentary);
    commentary_event["item"]["phase"] = serde_json::json!("commentary");
    let mock = responses::mount_sse_sequence(
        &server,
        vec![
            sse(vec![
                ev_custom_tool_call(
                    "selected", "exec",
                    r#"await tools.update_plan({plan:[{step:"Inspect",status:"in_progress"}]}); text("selected-" + "x".repeat(640) + "-end");"#,
                ),
                ev_completed("lookup"),
            ]),
            sse(vec![
                ev_custom_tool_call(
                    "unselected", "exec",
                    r#"text("ordinary-" + "y".repeat(640) + "-end");"#,
                ),
                ev_completed("ordinary"),
            ]),
            sse(vec![
                earlier_event,
                commentary_event,
                responses::ev_function_call("check", "exec_command", &arguments),
                ev_completed("actor"),
            ]),
            sse(vec![
                ev_assistant_message(
                    "assessment",
                    r#"{"risk_level":"high","user_authorization":"low","outcome":"deny"}"#,
                ),
                ev_completed("review"),
            ]),
            sse(vec![
                ev_assistant_message("result", "The check was not authorized."),
                ev_completed("done"),
            ]),
        ],
    )
    .await;
    let entry_limits = codex_guardian_context::ElasticSectionLimits::new;
    let mut policy = codex_guardian_context::ElasticBudgetConfig {
        sections: [
            (
                codex_guardian_context::ElasticSection::TranscriptMessages,
                entry_limits(ENTRY_TOKENS, ENTRY_TOKENS)?
                    .with_max_total_section_tokens(SECTION_TOKENS)?,
            ),
            (
                codex_guardian_context::ElasticSection::TranscriptTools,
                entry_limits(ENTRY_TOKENS, ENTRY_TOKENS)?
                    .with_max_total_section_tokens(SECTION_TOKENS)?,
            ),
        ]
        .into(),
        ..Default::default()
    };
    // Skills are absent in sync requests and release their entire reservation.
    let skills_minimum = codex_guardian_context::DEFAULT_MAX_INPUT_TOKENS
        - usize::try_from(policy.total_minimum_tokens())?
        + DEFAULT_SECTION_TOKENS
        - SELECTED_EXEC_TOKENS;
    policy.sections.insert(
        codex_guardian_context::ElasticSection::TrustedSkills,
        entry_limits(skills_minimum, DEFAULT_SECTION_TOKENS)?,
    );
    let settings = format!(
        r#"[features.guardianv2]
sync_elastic_budgeting = true
{}
[features.guardianv2.sync_elastic_sections.transcript_selected_exec_results]
nested_tool_names = ["update_plan"]
min_total_section_tokens = {SELECTED_EXEC_TOKENS}
max_section_entry_tokens = {SELECTED_EXEC_TOKENS}
max_total_section_tokens = {SELECTED_EXEC_TOKENS}
"#,
        toml::to_string(&policy)?
            .replace("[sections.", "[features.guardianv2.sync_elastic_sections.")
    );
    let test = test_codex()
        .with_cloud_config_bundle(
            CloudConfigBundleFixture::enterprise_config(settings).into_loader(),
        )
        .with_config(|config| {
            configure_scenario_catalog(config);
            config.features.enable(Feature::CodeMode).unwrap();
            config
                .features
                .enable(Feature::ExecutedToolCallMetadata)
                .unwrap();
            config.update_plan_enabled = true;
            config
                .features
                .enable(Feature::GuardianReuseParentCompaction)
                .unwrap();
            config.approvals_reviewer = ApprovalsReviewer::AutoReview;
            config.permissions.approval_policy =
                codex_core::config::Constrained::allow_any(AskForApproval::OnRequest);
        })
        .build_with_auto_env(&server)
        .await?;
    test.submit_text_turn(&user_request).await?;
    let requests = mock.requests();
    let guardian_text = requests[3].message_input_texts("user").join("\n");
    assert!(guardian_text.contains(&user_request));
    let transcript = guardian_text
        .split_once(">>> TRANSCRIPT START")
        .unwrap()
        .1
        .split_once(">>> TRANSCRIPT END")
        .unwrap()
        .0;
    assert!(!transcript.contains("Earlier untrusted"));
    assert!(transcript.contains("Untrusted assistant evidence"));
    let selected = format!("selected-{}-end", "x".repeat(640));
    assert!(transcript.contains(&selected), "{transcript}");
    assert!(!transcript.contains(&format!("ordinary-{}-end", "y".repeat(640))));
    assert!(
        requests.last().unwrap().body_json()["input"]
            .to_string()
            .contains(&earlier_commentary)
    );
    let mut snapshot = context_snapshot::format_request_history_snapshot(
        "Elastic sync budgeting keeps retained instructions, caps ordinary tool evidence, and reserves space for the exec output that ran the configured nested tool.",
        &requests,
        &ContextSnapshotOptions::default().rewrite_known_segments(),
    );
    for (pattern, replacement) in [
        (
            r"(?m)^(\s*Wall time) [0-9]+(?:\.[0-9]+)? seconds$",
            "$1 <DURATION> seconds",
        ),
        (r#"(?m)^(\s*"cwd": )"[^"]*""#, "$1\"<CWD>\""),
        (
            r#""command": \[\s*(?:"[^"]*",\s*)*("[^\n]*)\n\s*\]"#,
            "\"command\": [\"<SHELL>\", $1]",
        ),
        (
            r#"(?m)^(\s*"environment_id": )"(?:local|remote)""#,
            "$1\"<ENVIRONMENT>\"",
        ),
        (r#"(environment )"(?:local|remote)""#, "$1\"<ENVIRONMENT>\""),
    ] {
        snapshot = regex_lite::Regex::new(pattern)?
            .replace_all(&snapshot, replacement)
            .into_owned();
    }
    insta::assert_snapshot!("guardian_elastic_budgeting", snapshot);
    Ok(())
}

#[cfg(not(target_os = "windows"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guardian_code_mode_messaging_request_history() -> Result<()> {
    skip_if_no_network!(Ok(()));
    skip_if_wine_exec!(
        Ok(()),
        "Guardian approval actions require host-native paths"
    );
    let mut requests =
        super::guardian_subagent_authorization::code_mode_guardian_request_history().await?;
    let root =
        requests.first().expect("root model request").body_json()["client_metadata"]["thread_id"]
            .clone();
    requests.sort_by_key(|request| {
        let body = request.body_json();
        let metadata = &body["client_metadata"];
        if metadata["x-openai-subagent"] == "guardian" {
            2
        } else {
            usize::from(metadata["thread_id"] != root)
        }
    });
    let mut snapshot = context_snapshot::format_request_history_snapshot(
        "A root uses Code Mode to send a confirmation and receives a real user reply; a worker then requests an action that Guardian reviews with the confirmed question as assistant context. Independent model streams are grouped as root, worker, then Guardian.",
        &requests,
        &ContextSnapshotOptions::default().rewrite_known_segments(),
    );
    for (pattern, replacement) in [
        (
            r#"(?m)^(\s*"environment_id": )"(?:local|remote)""#,
            "$1\"<ENVIRONMENT>\"",
        ),
        (
            r#"(For this action on environment )"(?:local|remote)","#,
            "$1\"<ENVIRONMENT>\",",
        ),
        (r#"(?m)^(\s*"cwd": )"[^"]*""#, "$1\"<CWD>\""),
        (
            r#""command": \[\s*(?:"[^"]*",\s*)*"true"\s*\]"#,
            "\"command\": [\"<SHELL>\", \"true\"]",
        ),
        (
            r#"\{"type":"text","text":"Message sent\."\}"#,
            r#"{"text":"Message sent.","type":"text"}"#,
        ),
        (
            r#"\{"cmd":"true","sandbox_permissions":"require_escalated","justification":"Review the production deployment\."\}"#,
            r#"{"cmd":"true","justification":"Review the production deployment.","sandbox_permissions":"require_escalated"}"#,
        ),
    ] {
        snapshot = regex_lite::Regex::new(pattern)?
            .replace_all(&snapshot, replacement)
            .into_owned();
    }
    insta::assert_snapshot!("guardian_code_mode_messaging", snapshot);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn subagent_waits_for_its_inherited_environment_configuration() -> Result<()> {
    skip_if_no_network!(Ok(()));

    use super::remote_env::spawn_tests::PendingSpawnCase;
    use super::remote_env::spawn_tests::pending_subagent_scenario;

    let requests =
        pending_subagent_scenario(PendingSpawnCase::TurnSettings, configure_scenario_catalog)
            .await?;
    let labels = [
        "parent delegates while the environment is starting",
        "parent waits for the worker",
        "child sees the inherited starting environment",
        "child receives the ready result and updated environment",
        "parent receives the child result",
    ];
    assert_eq!(requests.len(), labels.len());
    let entries = requests
        .iter()
        .zip(labels)
        .map(|(request, label)| SnapshotEntry::body(request).labeled(label))
        .collect::<Vec<_>>();
    let snapshot = context_snapshot::format_context_snapshot(
        "A parent spawns a child before the selected environment is configured. The child sees it starting, waits, and receives the ready result after an ordinary parent turn-settings update.",
        &entries,
        &ContextSnapshotOptions::default(),
    );
    // Keep every request and its environment context; collapse unrelated guidance repeated per window.
    let mut snapshot = snapshot;
    for (pattern, replacement) in [
        (
            r"(?s)<permissions instructions>.*?</permissions instructions>",
            "<PERMISSIONS_INSTRUCTIONS>",
        ),
        (
            r"(?s)<environments_instructions>.*?</environments_instructions>",
            "<ENVIRONMENTS_INSTRUCTIONS>",
        ),
        (
            r"(?ms)(^\d+:message/developer:\n    You are (?:`/root`, the primary agent|an agent) in a team of agents[^\n]+)\n.*?(\n^\d+:message/developer:)",
            "${1}\n    <STANDARD_MULTI_AGENT_INSTRUCTIONS>${2}",
        ),
        (
            r#"(environment_id"\s*:\s*"|<environment id=")(?:local|remote)"#,
            "${1}<ENVIRONMENT>",
        ),
    ] {
        snapshot = regex_lite::Regex::new(pattern)?
            .replace_all(&snapshot, replacement)
            .into_owned();
    }
    // Windows guidance appears when the executor is ready, including under Wine.
    let snapshot_name =
        if core_test_support::test_target_os() == core_test_support::TestTargetOs::Windows {
            "subagent_inherits_pending_environment_windows"
        } else {
            "subagent_inherits_pending_environment"
        };
    insta::assert_snapshot!(snapshot_name, snapshot);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prediction_fork_inherits_parent_context() -> Result<()> {
    skip_if_no_network!(Ok(()));
    let requests = super::reasoning_effort_override::prediction_fork_requests().await?;
    insta::assert_snapshot!(context_snapshot::format_request_history_snapshot(
        "A parent pins high request effort, then selects low. Its ephemeral prediction fork inherits the context and baseline, then selects medium on its first turn. Continuing the parent at medium produces the same request.",
        &requests,
        &ContextSnapshotOptions::default().include_request_settings(),
    ));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prediction_fork_preserves_context_above_compaction_threshold() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let mock = mount_sse_sequence(
        &server,
        vec![
            sse(vec![
                ev_assistant_message("parent", "The release is ready for validation."),
                ev_completed_with_tokens("parent", /*total_tokens*/ 70_000),
            ]),
            sse(vec![
                ev_function_call_with_namespace(
                    "remaining",
                    "functions",
                    "get_context_remaining",
                    "{}",
                ),
                ev_completed_with_tokens("tool", /*total_tokens*/ 70_000),
            ]),
            sse(vec![
                ev_assistant_message("prediction", "Run the release checks."),
                ev_completed_with_tokens("prediction", /*total_tokens*/ 70_000),
            ]),
        ],
    )
    .await;
    let mut test = test_codex()
        .with_model("gpt-6-astra")
        .with_config(|config| {
            configure_scenario_catalog(config);
            config.workspace_roots = vec![config.cwd.clone()];
            config.model_auto_compact_token_limit = Some(50_000);
            config
                .features
                .enable(Feature::TokenBudget)
                .expect("enable token budget");
            config.token_budget = Some(TokenBudgetConfig {
                context_window_as_tool_output: true,
                reminder_threshold_tokens: Some(2_000),
                ..TokenBudgetConfig::default()
            });
        })
        .build_with_auto_env(&server)
        .await?;

    test.submit_turn("Prepare the release.").await?;
    test.codex.flush_rollout().await?;
    let (mut options, settings) = test
        .thread_manager
        .fork_options_from_parent(test.session_configured.thread_id)
        .await?;
    options.config.ephemeral = true;
    options.thread_extension_init.insert(AutoCompactionDisabled);
    let child = test
        .thread_manager
        .fork_legacy_thread(
            ForkSnapshot::Interrupted,
            options,
            test.codex.rollout_path().expect("parent rollout path"),
        )
        .await?;
    child.thread.restore_thread_settings(settings).await?;
    test.codex = child.thread;
    test.submit_turn("Suggest my next message.").await?;

    let requests = mock.requests();
    assert_eq!(requests.len(), 3);
    insta::assert_snapshot!(context_snapshot::format_request_history_snapshot(
        "The parent finishes above the compaction threshold. Its prediction fork retains the parent's context and reminder, but neither compacts nor adds another reminder after a tool call.",
        &requests,
        &ContextSnapshotOptions::default().include_request_settings(),
    ));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn attached_side_chat_preserves_model_context_on_cold_resume() -> Result<()> {
    use codex_context_fragments::SideConversationPolicy;
    use codex_history::InitialHistory;
    use codex_history::ResumedHistory;
    use codex_protocol::protocol::ThreadHistoryMode;
    use codex_thread_store::ForkBoundary;
    use codex_thread_store::LoadThreadHistoryParams;
    use codex_thread_store::PrepareForkParams;

    skip_if_no_network!(Ok(()));
    let server = start_mock_server().await;
    let mock = mount_sse_sequence(
        &server,
        [
            ("parent", "The migration is scheduled for Thursday."),
            ("side", "Stage a rollback before migration."),
            ("resumed", "Keep the rollback script close."),
        ]
        .into_iter()
        .map(|(id, reply)| sse(vec![ev_assistant_message(id, reply), ev_completed(id)]))
        .collect(),
    )
    .await;
    let mut test = test_codex()
        .with_model("gpt-6-astra")
        .with_history_mode(ThreadHistoryMode::Paginated)
        .with_config(|config| {
            configure_scenario_catalog(config);
            config.workspace_roots = vec![config.cwd.clone()];
            config.developer_instructions = Some("Use plain language.".to_string());
        })
        .build_with_auto_env(&server)
        .await?;
    test.submit_text_turn("Plan a database migration.").await?;
    test.codex.flush_rollout().await?;

    let prepared = test
        .thread_store
        .prepare_fork(PrepareForkParams {
            thread_id: test.session_configured.thread_id,
            boundary: ForkBoundary::Latest,
        })
        .await?;
    let mut options = test.start_thread_options().await;
    options.attached_thread = true;
    options.config.developer_instructions = Some(
        SideConversationPolicy::Attached
            .instructions(options.config.developer_instructions.as_deref()),
    );
    let mut resume_options = StartThreadOptions::new(options.config.clone());
    resume_options.environments = options.environments.clone();
    let child = test
        .thread_manager
        .fork_prepared_thread(options, prepared)
        .await?;
    child
        .thread
        .inject_response_items(SideConversationPolicy::Attached.fork_items())
        .await?;
    test.codex.shutdown_and_wait().await?;
    test.codex = child.thread;
    test.submit_text_turn("What is the safest rollback plan?")
        .await?;

    // Unload the child and resume from disk with the same policy as the TUI.
    test.codex.shutdown_and_wait().await?;
    test.thread_manager.remove_thread(&child.thread_id).await;
    let saved = test
        .thread_store
        .load_latest_model_context(LoadThreadHistoryParams {
            thread_id: child.thread_id,
            include_archived: false,
        })
        .await?;
    resume_options.initial_history = InitialHistory::Resumed(ResumedHistory {
        history_revision: saved.revision,
        conversation_id: saved.thread_id,
        history: Arc::new(saved.items),
        rollout_path: test.codex.rollout_path(),
    });
    resume_options.client_mcp_extensions = test.codex.client_mcp_extensions();
    test.codex = test
        .thread_manager
        .start_thread(resume_options)
        .await?
        .thread;
    test.submit_text_turn("What did you recommend before I closed the chat?")
        .await?;

    let requests = mock.requests();
    assert_eq!(requests.len(), 3);
    insta::assert_snapshot!(context_snapshot::format_request_history_snapshot(
        "A side chat receives the parent turn as reference, adds its own turn, and resumes from disk with the policy and prior answer intact.",
        &requests,
        &ContextSnapshotOptions::default(),
    ));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guardian_checkpoint_migration_request_history() -> Result<()> {
    skip_if_no_network!(Ok(()));
    use super::guardian_checkpoint_migration::migration_scenario;
    let requests = migration_scenario().await?;
    let mut snapshot = context_snapshot::format_request_history_snapshot(
        "An old checkpoint retains a user restriction and verified answer. Incompatible automatic compaction keeps legacy review across restart with its saved answer; compatible manual compaction immediately activates thread-owned review.",
        &requests,
        &ContextSnapshotOptions::default()
            .rewrite_known_segments()
            .include_request_settings(),
    );
    // Normalize executor paths and shell wrappers in the reviewed actions.
    for (pattern, replacement) in [
        (r#"(?m)^(\s*"cwd": )"[^"]*""#, "$1\"<CWD>\""),
        (
            r#""command": \[\s*(?:"[^"]*",\s*)*"exit 0"\s*\]"#,
            "\"command\": [\"<SHELL>\", \"exit 0\"]",
        ),
    ] {
        snapshot = regex_lite::Regex::new(pattern)?
            .replace_all(&snapshot, replacement)
            .into_owned();
    }
    insta::assert_snapshot!("guardian_checkpoint_migration", snapshot);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn app_tool_exposure_request_history() -> Result<()> {
    let requests = super::app_tool_exposure::connector_exposure_requests(
        super::app_tool_exposure::ExposureCase::non_deferred(
            codex_protocol::openai_models::ToolMode::CodeModeOnly,
        ),
    )
    .await?;
    insta::assert_snapshot!(
        "app_tool_exposure_CodeModeOnly",
        context_snapshot::format_request_history_snapshot(
            "A non-deferred connector is called through code mode while another connector stays deferred.",
            &requests,
            &ContextSnapshotOptions::default()
                .rewrite_known_segments()
                .include_request_settings(),
        )
    );
    Ok(())
}
