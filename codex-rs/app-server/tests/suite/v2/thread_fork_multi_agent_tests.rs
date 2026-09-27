//! Regression coverage for preserving the source runtime across empty fork cutoffs.

use anyhow::Result;
use app_test_support::MockResponsesConfig;
use app_test_support::TestAppServer;
use app_test_support::create_mock_responses_server_repeating_assistant;
use app_test_support::write_models_cache_with_models;
use codex_app_server_protocol::ApprovalsReviewer;
use codex_app_server_protocol::AskForApproval;
use codex_app_server_protocol::ClientRequest;
use codex_app_server_protocol::SandboxMode;
use codex_app_server_protocol::ThreadForkParams;
use codex_app_server_protocol::ThreadForkResponse;
use codex_app_server_protocol::ThreadHistoryMode;
use codex_app_server_protocol::ThreadResumeParams;
use codex_app_server_protocol::ThreadResumeResponse;
use codex_app_server_protocol::ThreadStartParams;
use codex_app_server_protocol::TurnStartParams;
use codex_app_server_protocol::UserInput;
use codex_features::Feature;
use codex_protocol::protocol::MultiAgentVersion;
use codex_rollout::read_session_meta_line;
use core_test_support::load_default_config_for_test;
use pretty_assertions::assert_eq;
use serde_json::Value;
use tempfile::TempDir;

enum SourceState {
    Loaded,
    Unloaded,
}

#[test_case::test_case(ThreadHistoryMode::Legacy, SourceState::Loaded; "legacy_loaded")]
#[test_case::test_case(ThreadHistoryMode::Legacy, SourceState::Unloaded; "legacy_unloaded")]
#[test_case::test_case(ThreadHistoryMode::Paginated, SourceState::Loaded; "paginated_loaded")]
#[test_case::test_case(ThreadHistoryMode::Paginated, SourceState::Unloaded; "paginated_unloaded")]
#[tokio::test]
async fn fork_before_first_turn_preserves_model_selected_multi_agent_version(
    history_mode: ThreadHistoryMode,
    source_state: SourceState,
) -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    MockResponsesConfig::new(&server.uri())
        .disable_feature(Feature::MultiAgentV2)
        .write(codex_home.path())?;
    let config = load_default_config_for_test(&codex_home).await;
    let mut model = codex_core::test_support::construct_model_info_offline("mock-model", &config);
    model.multi_agent_version = Some(MultiAgentVersion::V2);
    write_models_cache_with_models(codex_home.path(), vec![model]).await?;
    let mut mcp = TestAppServer::builder()
        .with_codex_home(codex_home.path())
        .build_initialized()
        .await?;
    let parent = mcp
        .start_thread(ThreadStartParams {
            history_mode: Some(history_mode),
            ..Default::default()
        })
        .await?
        .thread;
    let completed = mcp
        .start_turn_and_wait_for_completion(TurnStartParams {
            thread_id: parent.id.clone(),
            input: vec![UserInput::Text {
                text: "First message".to_string(),
                text_elements: Vec::new(),
            }],
            ..Default::default()
        })
        .await?;
    if matches!(source_state, SourceState::Unloaded) {
        mcp.shutdown_gracefully().await?;
        mcp = TestAppServer::builder()
            .with_codex_home(codex_home.path())
            .build_initialized()
            .await?;
    }
    let ThreadForkResponse { thread: child, .. } = mcp
        .request(|request_id| ClientRequest::ThreadFork {
            request_id,
            params: ThreadForkParams {
                thread_id: parent.id,
                before_turn_id: Some(completed.turn.id),
                exclude_turns: true,
                // Version recovery must also work when no permission settings need restoring.
                approval_policy: Some(AskForApproval::Never),
                approvals_reviewer: Some(ApprovalsReviewer::User),
                sandbox: Some(SandboxMode::ReadOnly),
                ..Default::default()
            },
        })
        .await?;
    assert_eq!(
        read_session_meta_line(child.path.as_ref().expect("fork rollout").as_path())
            .await?
            .meta
            .multi_agent_version,
        Some(MultiAgentVersion::V2)
    );

    // Resume before the child's first turn can persist another version-bearing context.
    mcp.shutdown_gracefully().await?;
    mcp = TestAppServer::builder()
        .with_codex_home(codex_home.path())
        .build_initialized()
        .await?;
    let _: ThreadResumeResponse = mcp
        .request(|request_id| ClientRequest::ThreadResume {
            request_id,
            params: ThreadResumeParams {
                thread_id: child.id.clone(),
                exclude_turns: true,
                ..Default::default()
            },
        })
        .await?;
    mcp.start_turn_and_wait_for_completion(TurnStartParams {
        thread_id: child.id,
        input: vec![UserInput::Text {
            text: "Forked message".to_string(),
            text_elements: Vec::new(),
        }],
        ..Default::default()
    })
    .await?;

    let requests = server.received_requests().await.expect("response requests");
    let mut multi_agent_namespaces = Vec::new();
    for request in requests
        .iter()
        .filter(|request| request.url.path().ends_with("/responses"))
    {
        let body = request.body_json::<Value>()?;
        multi_agent_namespaces.push(
            body["tools"]
                .as_array()
                .expect("tools")
                .iter()
                .filter_map(|tool| tool["name"].as_str())
                .filter(|name| matches!(*name, "collaboration" | "multi_agent_v1"))
                .map(str::to_owned)
                .collect::<Vec<_>>(),
        );
    }
    assert_eq!(
        multi_agent_namespaces,
        vec![vec!["collaboration"], vec!["collaboration"]]
    );
    Ok(())
}
