//! Exercises model policy, legacy config, and live user modes through the app-server API.

use super::*;
use codex_protocol::openai_models::GuardianModelPolicy;
use codex_protocol::openai_models::GuardianReviewMode;
use pretty_assertions::assert_eq;
use test_case::test_case;

#[derive(Clone, Copy)]
enum ReviewConstraint {
    Ordinary,
    Sensitive,
}

#[derive(Clone, Copy)]
enum UserMode {
    User,
    Automatic,
    FullAccess,
}

#[test_case(Some(GuardianReviewMode::Adaptive), UserMode::Automatic, false, 1, 0, ReviewConstraint::Ordinary; "model enables async without legacy flag")]
#[test_case(Some(GuardianReviewMode::Synchronous), UserMode::Automatic, true, 0, 1, ReviewConstraint::Ordinary; "model disables async despite legacy flag")]
#[test_case(Some(GuardianReviewMode::Disabled), UserMode::Automatic, true, 0, 0, ReviewConstraint::Ordinary; "model disables ordinary cua review")]
#[test_case(Some(GuardianReviewMode::Adaptive), UserMode::User, true, 0, 0, ReviewConstraint::Ordinary; "user mode never scores")]
#[test_case(Some(GuardianReviewMode::Adaptive), UserMode::FullAccess, true, 0, 0, ReviewConstraint::Ordinary; "full access never scores")]
#[test_case(Some(GuardianReviewMode::Adaptive), UserMode::FullAccess, true, 0, 0, ReviewConstraint::Sensitive; "full access skips sensitive action reviews")]
#[test_case(None, UserMode::Automatic, false, 0, 1, ReviewConstraint::Ordinary; "legacy synchronous config")]
#[test_case(None, UserMode::Automatic, true, 1, 0, ReviewConstraint::Ordinary; "legacy adaptive config")]
#[test_case(Some(GuardianReviewMode::Adaptive), UserMode::Automatic, false, 1, 1, ReviewConstraint::Sensitive; "sensitive elicitation cannot use initial cua allowance")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn model_guardian_policy_controls_cua(
    mode: Option<GuardianReviewMode>,
    user_mode: UserMode,
    legacy_enabled: bool,
    expected_scores: usize,
    expected_reviews: usize,
    constraint: ReviewConstraint,
) -> Result<()> {
    skip_if_no_network!(Ok(()));
    let state = Arc::new(MockResponsesState {
        mcp_server_name: Some("node_repl"),
        mcp_tool_sequence: Some(&["js"]),
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
    let (mcp_url, mcp_server) = start_mcp_server_with_tools(
        &["js"],
        matches!(constraint, ReviewConstraint::Sensitive).then_some(/*t*/ true),
    )
    .await?;
    let codex_home = TempDir::new()?;
    MockResponsesConfig::new(&responses_url)
        .with_model(MODEL)
        .with_provider_config("supports_websockets = false")
        .with_approval_policy("on-request")
        .with_root_config("approvals_reviewer = \"auto_review\"")
        .with_extra_config(&format!(
            "[mcp_servers.node_repl]\nurl = \"{mcp_url}/mcp\"\ndefault_tools_approval_mode = \"auto\"\n\n[features.guardianv2]\nenabled = {legacy_enabled}"
        ))
        .enable_feature(Feature::GuardianApproval)
        .write(codex_home.path())?;
    let config = load_default_config_for_test(&codex_home).await;
    let mut model = codex_core::test_support::construct_model_info_offline(MODEL, &config);
    // Exercise both directions of precedence over the legacy CUA bit.
    model.node_repl_auto_review_required =
        mode.is_none() || mode == Some(GuardianReviewMode::Disabled);
    model.guardian = mode.map(|computer_use| GuardianModelPolicy {
        computer_use: Some(computer_use),
        ..Default::default()
    });
    write_models_cache_with_models(codex_home.path(), vec![model]).await?;
    let mut app_server = TestAppServer::builder()
        .with_codex_home(codex_home.path())
        .build_initialized_with_timeout(TIMEOUT)
        .await?;
    let started = app_server
        .start_thread(ThreadStartParams {
            approvals_reviewer: Some(match user_mode {
                UserMode::User => ApprovalsReviewer::User,
                UserMode::Automatic | UserMode::FullAccess => ApprovalsReviewer::AutoReview,
            }),
            approval_policy: matches!(user_mode, UserMode::FullAccess)
                .then_some(AskForApproval::Never),
            sandbox: matches!(user_mode, UserMode::FullAccess)
                .then_some(SandboxMode::DangerFullAccess),
            ..Default::default()
        })
        .await?;
    let thread = started.thread;
    if expected_scores > 0 {
        timeout(TIMEOUT, async {
            while state.luna_connections.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await?;
    }
    state.allow_guardian_review.notify_one();
    state.allow_luna.notify_one();
    let id = app_server
        .send_turn_start_request(TurnStartParams {
            thread_id: thread.id.clone(),
            input: vec![UserInput::Text {
                text: USER_CONTEXT.to_owned(),
                text_elements: Vec::new(),
            }],
            ..Default::default()
        })
        .await?;
    let _: TurnStartResponse = timeout(TIMEOUT, app_server.read_response(id)).await??;
    let completed: TurnCompletedNotification =
        timeout(TIMEOUT, app_server.read_notification("turn/completed")).await??;
    assert_eq!(completed.turn.status, TurnStatus::Completed);
    if expected_scores > 0 {
        wait_for_luna_request(&state, expected_scores - 1).await?;
    }
    app_server.shutdown_gracefully().await?;
    assert_eq!(
        (
            state.luna_requests.lock().expect("score requests").len(),
            state.guardian_reviews.load(Ordering::SeqCst),
            state.luna_connections.load(Ordering::SeqCst) > 0,
        ),
        (expected_scores, expected_reviews, expected_scores > 0)
    );
    mcp_server.abort();
    responses_server.abort();
    Ok(())
}
