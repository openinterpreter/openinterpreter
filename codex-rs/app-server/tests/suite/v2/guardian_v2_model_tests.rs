//! Verifies model-required CUA scoring across model changes in a live thread.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::Ordering;

use anyhow::Result;
use app_test_support::MockResponsesConfig;
use app_test_support::TestAppServer;
use app_test_support::write_models_cache_with_models;
use axum::Router;
use axum::routing::get;
use codex_app_server_protocol::ApprovalsReviewer;
use codex_app_server_protocol::ThreadStartParams;
use codex_app_server_protocol::ThreadStartResponse;
use codex_app_server_protocol::TurnCompletedNotification;
use codex_app_server_protocol::TurnStartParams;
use codex_app_server_protocol::TurnStartResponse;
use codex_app_server_protocol::TurnStatus;
use codex_app_server_protocol::UserInput;
use codex_features::Feature;
use core_test_support::load_default_config_for_test;
use core_test_support::skip_if_no_network;
use pretty_assertions::assert_eq;
use tempfile::TempDir;
use test_case::test_case;
use tokio::net::TcpListener;
use tokio::time::timeout;

use super::MODEL;
use super::MockResponsesState;
use super::TIMEOUT;
use super::USER_CONTEXT;
use super::luna_websocket;
use super::parent_response;
use super::start_mcp_server_with_tools;
use super::wait_for_luna_request;

#[test_case("node_repl"; "browser")]
#[test_case("cua_repl"; "computer use")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn computer_use_scoring_follows_model_review_requirement(
    server_name: &'static str,
) -> Result<()> {
    skip_if_no_network!(Ok(()));

    const REVIEWED_MODEL: &str = "reviewed-model";
    let state = Arc::new(MockResponsesState {
        mcp_server_name: Some(server_name),
        mcp_tool_sequence: Some(&["js"]),
        mcp_messages: Mutex::new(vec!["hello"]),
        ..Default::default()
    });
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let responses_url = format!("http://{}", listener.local_addr()?);
    let router = Router::new()
        .route("/v1/responses", get(luna_websocket).post(parent_response))
        .with_state(Arc::clone(&state));
    let responses_server = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    let (mcp_url, mcp_server) =
        start_mcp_server_with_tools(&["js"], /*sensitive_action*/ None).await?;
    let codex_home = TempDir::new()?;
    MockResponsesConfig::new(&responses_url)
        .with_model(MODEL)
        .with_provider_config("supports_websockets = false")
        .with_approval_policy("on-request")
        .with_root_config("approvals_reviewer = \"auto_review\"")
        .with_extra_config(&format!(
            "[mcp_servers.{server_name}]\nurl = \"{mcp_url}/mcp\"\ndefault_tools_approval_mode = \"auto\"\n\n[features.guardianv2]\nenabled = true"
        ))
        .enable_feature(Feature::GuardianApproval)
        .write(codex_home.path())?;
    let config = load_default_config_for_test(&codex_home).await;
    let ordinary_model = codex_core::test_support::construct_model_info_offline(MODEL, &config);
    let mut reviewed_model =
        codex_core::test_support::construct_model_info_offline(REVIEWED_MODEL, &config);
    reviewed_model.node_repl_auto_review_required = true;
    write_models_cache_with_models(codex_home.path(), vec![ordinary_model, reviewed_model]).await?;
    let mut app_server = TestAppServer::builder()
        .with_codex_home(codex_home.path())
        .build_initialized_with_timeout(TIMEOUT)
        .await?;
    let request_id = app_server
        .send_thread_start_request_with_auto_env(ThreadStartParams {
            approvals_reviewer: Some(ApprovalsReviewer::AutoReview),
            ..Default::default()
        })
        .await?;
    let thread: ThreadStartResponse =
        timeout(TIMEOUT, app_server.read_response(request_id)).await??;

    state.allow_guardian_review.notify_one();
    for (model, expected_samples) in [
        (MODEL, 0),
        (REVIEWED_MODEL, 1),
        (MODEL, 1),
        (REVIEWED_MODEL, 2),
    ] {
        state.parent_requests.store(0, Ordering::SeqCst);
        app_server.clear_message_buffer();
        state.allow_luna.notify_one();
        let request_id = app_server
            .send_turn_start_request(TurnStartParams {
                thread_id: thread.thread.id.clone(),
                model: Some(model.to_owned()),
                input: vec![UserInput::Text {
                    text: USER_CONTEXT.to_owned(),
                    text_elements: Vec::new(),
                }],
                ..Default::default()
            })
            .await?;
        let _: TurnStartResponse = timeout(TIMEOUT, app_server.read_response(request_id)).await??;
        let completed: TurnCompletedNotification =
            timeout(TIMEOUT, app_server.read_notification("turn/completed")).await??;
        assert_eq!(completed.turn.status, TurnStatus::Completed);
        if model == REVIEWED_MODEL {
            wait_for_luna_request(&state, expected_samples - 1).await?;
        }
        assert_eq!(
            state
                .luna_requests
                .lock()
                .expect("luna requests lock")
                .len(),
            expected_samples,
            "only models requiring REPL review should send classifier requests"
        );
    }

    app_server.shutdown_gracefully().await?;
    mcp_server.abort();
    responses_server.abort();
    Ok(())
}
