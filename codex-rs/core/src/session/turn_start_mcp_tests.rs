//! Turn-start MCP reconciliation and cooperative discovery cancellation regressions.

use super::*;
use codex_mcp::McpResourceClient;
use codex_mcp::McpResourceClientCacheKey;
use pretty_assertions::assert_eq;

struct Recorder {
    client: McpResourceClient,
    requires_mcp: std::sync::atomic::AtomicBool,
    keys: std::sync::Mutex<Vec<McpResourceClientCacheKey>>,
    discovered: tokio::sync::Notify,
}

impl codex_extension_api::TurnLifecycleContributor for Recorder {
    fn turn_start_phase(
        &self,
        _thread_store: &codex_extension_api::ExtensionData,
    ) -> codex_extension_api::TurnStartPhase {
        codex_extension_api::TurnStartPhase::RegularTaskStart
    }

    fn requires_mcp_runtime(&self, _thread_store: &codex_extension_api::ExtensionData) -> bool {
        self.requires_mcp.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn on_turn_start<'a>(
        &'a self,
        _input: codex_extension_api::TurnStartInput<'a>,
    ) -> codex_extension_api::ExtensionFuture<'a, ()> {
        Box::pin(async move {
            self.keys
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(self.client.cache_key());
            self.discovered.notify_one();
            std::future::pending::<()>().await;
        })
    }
}

struct RejectingTurnGate(std::sync::Arc<tokio::sync::Notify>);

impl codex_extension_api::TurnLifecycleContributor for RejectingTurnGate {
    fn turn_start_gate<'a>(
        &'a self,
        _input: codex_extension_api::TurnStartInput<'a>,
    ) -> codex_extension_api::ExtensionFuture<'a, Result<(), String>> {
        let called = std::sync::Arc::clone(&self.0);
        Box::pin(async move {
            called.notify_one();
            Err("managed mission document is unavailable".to_string())
        })
    }
}

#[tokio::test]
async fn regular_task_rejects_failed_turn_gate_before_turn_started() {
    let (mut session, turn_context, events) = make_session_and_context_with_rx().await;
    let called = std::sync::Arc::new(tokio::sync::Notify::new());
    let mut builder = codex_extension_api::ExtensionRegistryBuilder::new();
    builder.turn_lifecycle_contributor(std::sync::Arc::new(RejectingTurnGate(Arc::clone(&called))));
    Arc::get_mut(&mut session)
        .expect("session should be uniquely owned before spawning")
        .services
        .extensions = Arc::new(builder.build());

    session
        .spawn_task(
            Arc::clone(&turn_context),
            Vec::new(),
            crate::tasks::RegularTask::new(),
        )
        .await;
    tokio::time::timeout(std::time::Duration::from_secs(5), called.notified())
        .await
        .expect("regular task should invoke the turn admission gate");

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(250);
    let mut saw_turn_started = false;
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(
            std::cmp::min(remaining, std::time::Duration::from_millis(25)),
            events.recv(),
        )
        .await
        {
            Ok(Ok(event)) => {
                if matches!(
                    event.msg,
                    codex_protocol::protocol::EventMsg::TurnStarted(_)
                ) {
                    saw_turn_started = true;
                    break;
                }
            }
            Ok(Err(_)) | Err(_) => break,
        }
    }
    assert!(
        !saw_turn_started,
        "a rejected admission must not emit TurnStarted or enter model execution"
    );
    session
        .abort_all_tasks(codex_protocol::protocol::TurnAbortReason::Interrupted)
        .await;
}

#[tokio::test]
async fn turn_start_refreshes_dirty_mcp_reuses_clean_runtime_and_cancels_discovery() {
    let (mut session, turn_context) = make_session_and_context().await;
    let recorder = Arc::new(Recorder {
        client: McpResourceClient::new(Arc::clone(&session.services.mcp_runtime)),
        requires_mcp: Default::default(),
        keys: Default::default(),
        discovered: Default::default(),
    });
    let initial_key = recorder.client.cache_key();
    let mut builder = codex_extension_api::ExtensionRegistryBuilder::new();
    builder.turn_lifecycle_contributor(recorder.clone());
    session.services.extensions = Arc::new(builder.build());
    let session = Arc::new(session);
    let turn_context = Arc::new(turn_context);

    for turn in 0..4 {
        recorder
            .requires_mcp
            .store(turn > 0, std::sync::atomic::Ordering::Relaxed);
        if turn == 0 {
            session.mark_mcp_runtime_dirty(McpRefreshReason::ExplicitInvalidation);
        } else if turn == 3 {
            session.services.mcp_runtime.reconnect_on_next_refresh();
            session.request_mcp_runtime_refresh(McpRefreshReason::ExplicitInvalidation);
        }
        session
            .spawn_task(
                Arc::clone(&turn_context),
                Vec::new(),
                crate::tasks::RegularTask::new(),
            )
            .await;
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            recorder.discovered.notified(),
        )
        .await
        .expect("regular-task contributor should run");
        assert_eq!(session.mcp_refresh.is_pending(), turn == 0);
        assert_eq!(
            recorder
                .keys
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .len(),
            turn + 1
        );
        if turn > 0 {
            session.refresh_mcp_if_dirty().await;
            assert!(
                recorder.client.cache_key()
                    == recorder
                        .keys
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)[turn],
                "a clean refresh must reuse the published MCP runtime"
            );
        }
        let (cancellation_token, done) = {
            let active_turn = session.active_turn.lock().await;
            let task = active_turn
                .as_ref()
                .and_then(|turn| turn.task.as_ref())
                .expect("discovery should run inside the registered task");
            (task.cancellation_token.clone(), Arc::clone(&task.done))
        };
        let completed = done.notified();
        tokio::pin!(completed);
        completed.as_mut().enable();
        cancellation_token.cancel();
        tokio::time::timeout(std::time::Duration::from_secs(5), completed)
            .await
            .expect("discovery cancellation must finish the task without forced abort");
        assert!(
            session.mcp_refresh.is_pending(),
            "cancelled discovery must request an MCP refresh before the task completes"
        );
        session.abort_all_tasks(TurnAbortReason::Interrupted).await;
    }
    let keys = recorder
        .keys
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert!(
        keys[0] == initial_key,
        "unrelated contributors must not reconcile MCP"
    );
    assert!(
        keys[1] != initial_key,
        "discovery must observe the reconciled generation"
    );
    assert!(
        keys[2] != keys[1],
        "cancelled cloud preparation must request reprojection on the next turn"
    );
    assert!(
        keys[3] != keys[2],
        "the next turn must consume a pending reconnect before discovery"
    );
}

#[derive(Clone, Copy)]
enum McpPause {
    Start,
    Result,
}

struct PauseMcpCall {
    phase: McpPause,
    entered: Notify,
    release: Notify,
    outcome: std::sync::OnceLock<codex_extension_api::ToolCallOutcome>,
}

impl codex_extension_api::ToolLifecycleContributor for PauseMcpCall {
    fn on_tool_start<'a>(
        &'a self,
        input: codex_extension_api::ToolStartInput<'a>,
    ) -> codex_extension_api::ToolLifecycleFuture<'a> {
        Box::pin(async move {
            assert_eq!(input.mcp_tool.unwrap().tool_info().tool.name, "probe");
            if matches!(self.phase, McpPause::Start) {
                self.entered.notify_one();
                self.release.notified().await;
            }
        })
    }

    fn on_mcp_tool_result<'a>(
        &'a self,
        _input: codex_extension_api::McpToolResultInput<'a>,
    ) -> codex_extension_api::ToolLifecycleFuture<'a> {
        Box::pin(async move {
            if matches!(self.phase, McpPause::Result) {
                self.entered.notify_one();
                self.release.notified().await;
            }
        })
    }

    fn on_tool_finish<'a>(
        &'a self,
        input: codex_extension_api::ToolFinishInput<'a>,
    ) -> codex_extension_api::ToolLifecycleFuture<'a> {
        Box::pin(async move { self.outcome.set(input.outcome).expect("one finish") })
    }
}

#[test_case::test_case(McpPause::Start; "stop_before_send")]
#[test_case::test_case(McpPause::Result; "stop_after_send")]
#[tokio::test]
async fn mcp_budget_stop_at_handoff(phase: McpPause) -> anyhow::Result<()> {
    use crate::tools::parallel::ToolCallRuntime;
    use crate::tools::router::ToolCall;
    use codex_extension_api::ToolCallOutcome;
    use core_test_support::hooks::mount_hook_mcp;
    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::Ordering;

    core_test_support::skip_if_no_network!(Ok(()));
    let endpoint = wiremock::MockServer::start().await;
    let calls = Arc::new(AtomicUsize::new(0));
    let received = Arc::clone(&calls);
    mount_hook_mcp(&endpoint, "handoff", &["probe"], move |_| {
        received.fetch_add(1, Ordering::Relaxed);
        json!({"content": [{"type":"text", "text":"peer result"}]})
    })
    .await;
    let observer = Arc::new(PauseMcpCall {
        phase,
        entered: Notify::new(),
        release: Notify::new(),
        outcome: Default::default(),
    });
    let (mut session, _) = make_session_and_context().await;
    let mut extensions = codex_extension_api::ExtensionRegistryBuilder::new();
    extensions.tool_lifecycle_contributor(observer.clone());
    session.services.extensions = Arc::new(extensions.build());
    session
        .rollout_budget()
        .configure(crate::config::RolloutBudgetConfig {
            enforce_limit: true,
            limit_budget: 1.0,
            reminder_at_context_start: true,
            reminder_at_remaining_tokens: Vec::new(),
            sampling_token_weight: 1.0,
            prefill_token_weight: 1.0,
        });
    {
        let mut state = session.state.lock().await;
        let config = Arc::make_mut(&mut state.session_configuration.original_config_do_not_use);
        config
            .mcp_servers
            .set(serde_json::from_value(json!({"handoff": {
                "url": format!("{}/repl", endpoint.uri()),
                "default_tools_approval_mode": "approve"
            }}))?)?;
    }
    let session = Arc::new(session);
    session.mark_mcp_runtime_dirty(McpRefreshReason::ExplicitInvalidation);
    session
        .wait_for_mcp_server(&codex_mcp::McpServerKey::unscoped("handoff"))
        .await;
    let turn = session.new_default_turn().await;
    let cancel = CancellationToken::new();
    let step = session
        .capture_step_context_with_required_mcp_servers(
            turn,
            &cancel,
            &["handoff".to_owned()],
            &HashSet::new(),
        )
        .await?;
    let tool = step
        .mcp
        .tools()
        .iter()
        .find(|tool| tool.server_name() == "handoff" && tool.tool.name == "probe")
        .expect("MCP server advertised probe");
    let call = ToolCall {
        tool_name: tool.canonical_tool_name(),
        call_id: "handoff-probe".into(),
        payload: ToolPayload::Function {
            arguments: "{}".into(),
        },
        encrypted_function_args: None,
        native_call: None,
    };
    let runtime = ToolCallRuntime::new(
        session.clone(),
        step,
        Arc::new(tokio::sync::Mutex::new(TurnDiffTracker::new())),
    );
    let response = runtime.handle_tool_call(call, cancel);
    tokio::pin!(response);
    tokio::select! {
        () = observer.entered.notified() => {}
        reply = &mut response => anyhow::bail!("MCP call finished before the observer paused: {reply:?}"),
    }
    let usage = codex_protocol::protocol::TokenUsage {
        output_tokens: 2,
        ..Default::default()
    };
    session
        .record_rollout_budget_usage("peer-turn", &usage.into())
        .await?;
    observer.release.notify_one();
    let reply = response.await?;
    let (count, outcome) = match phase {
        McpPause::Start => {
            assert_eq!(
                reply.item,
                ResponseInputItem::FunctionCallOutput {
                    call_id: "handoff-probe".into(),
                    output: FunctionCallOutputPayload::from_text(
                        "aborted before MCP handoff: rollout budget exhausted".into()
                    ),
                }
                .into()
            );
            (0, ToolCallOutcome::Aborted)
        }
        McpPause::Result => (1, ToolCallOutcome::Completed { success: true }),
    };
    assert_eq!(calls.load(Ordering::Relaxed), count);
    assert_eq!(observer.outcome.get(), Some(&outcome));
    session.services.mcp_runtime.shutdown().await;
    Ok(())
}
