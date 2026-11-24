//! Exercises origin-bound approval replies through real WebSocket workers.

use super::*;
use crate::app::app_server_requests::BufferedServerRequest;
use crate::app_server_session::AppServerSession;
use crate::app_server_session::ThreadParamsMode;
use codex_app_server_client::AppServerEvent;
use codex_app_server_protocol::CommandExecutionApprovalDecision;
use codex_app_server_protocol::CommandExecutionRequestApprovalParams;
use codex_app_server_protocol::FileChangeApprovalDecision;
use codex_app_server_protocol::FileChangeRequestApprovalParams;
use codex_app_server_protocol::JSONRPCMessage;
use codex_app_server_protocol::JSONRPCResponse;
use codex_protocol::ThreadId;
use futures::SinkExt;
use futures::StreamExt;
use pretty_assertions::assert_eq;
use serde_json::json;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::Duration;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;

async fn connection() -> color_eyre::Result<(
    AppServerSession,
    mpsc::UnboundedReceiver<JSONRPCMessage>,
    JoinHandle<color_eyre::Result<()>>,
)> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let endpoint = crate::resolve_remote_addr(&format!("ws://{}", listener.local_addr()?))?;
    let (observed, receiver) = mpsc::unbounded_channel();
    let task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await?;
        let mut socket = tokio_tungstenite::accept_async(stream).await?;
        while let Some(frame) = socket.next().await {
            let Message::Text(text) = frame? else {
                continue;
            };
            let message: JSONRPCMessage = serde_json::from_str(&text)?;
            match message {
                JSONRPCMessage::Request(request) if request.method == "initialize" => {
                    socket
                        .send(Message::Text(
                            json!({"id":request.id,"result":{
                                "userAgent":"retained-reply-test","codexHome":std::env::temp_dir(),
                                "platformFamily":std::env::consts::FAMILY,
                                "platformOs":std::env::consts::OS
                            }})
                            .to_string()
                            .into(),
                        ))
                        .await?;
                }
                JSONRPCMessage::Notification(_) => {}
                message => observed.send(message)?,
            }
        }
        Ok(())
    });
    let client = crate::connect_remote_app_server(endpoint).await?;
    Ok((
        AppServerSession::new(
            crate::backend_id::BackendId::new(),
            client,
            ThreadParamsMode::Remote,
        ),
        receiver,
        task,
    ))
}

fn patch_request(thread_id: ThreadId) -> ServerRequest {
    ServerRequest::FileChangeRequestApproval {
        request_id: AppServerRequestId::Integer(7),
        params: FileChangeRequestApprovalParams {
            thread_id: thread_id.to_string(),
            turn_id: "turn".into(),
            item_id: "patch".into(),
            started_at_ms: 0,
            reason: None,
            grant_root: None,
        },
    }
}

fn command_request(thread_id: ThreadId) -> ServerRequest {
    ServerRequest::CommandExecutionRequestApproval {
        request_id: AppServerRequestId::Integer(7),
        params: CommandExecutionRequestApprovalParams {
            kind: Default::default(),
            thread_id: thread_id.to_string(),
            turn_id: "turn".into(),
            item_id: "command".into(),
            started_at_ms: 0,
            approval_id: None,
            environment_id: None,
            reason: None,
            network_approval_context: None,
            command: Some("echo hello".into()),
            cwd: None,
            command_actions: None,
            additional_permissions: None,
            proposed_execpolicy_amendment: None,
            proposed_network_policy_amendments: None,
            available_decisions: None,
        },
    }
}

#[tokio::test]
async fn direct_and_buffered_colliding_ids_reply_on_their_origin() -> color_eyre::Result<()> {
    let (mut app, _events, _ops) = crate::app::tests::make_test_app_with_channels().await;
    let (command_server, mut command_replies, command_task) = connection().await?;
    let (mut patch_server, mut patch_replies, patch_task) = connection().await?;
    let command_thread = ThreadId::new();
    let patch_thread = ThreadId::new();
    app.primary_thread_id = Some(command_thread);
    app.active_thread_id = Some(command_thread);
    app.ensure_thread_channel(command_thread);
    app.ensure_thread_channel(patch_thread);
    app.agents_overview
        .dispatched_requests
        .entry(patch_thread)
        .or_default();

    app.handle_app_server_event(
        &command_server,
        AppServerEvent::ServerRequest(Box::new(command_request(command_thread))),
    )
    .await;
    app.handle_app_server_event(
        &patch_server,
        AppServerEvent::ServerRequest(Box::new(patch_request(patch_thread))),
    )
    .await;

    app.submit_thread_op(
        &mut patch_server,
        command_thread,
        AppCommand::ExecApproval {
            id: "command".into(),
            turn_id: None,
            decision: CommandExecutionApprovalDecision::Decline,
        },
    )
    .await?;
    app.primary_thread_id = Some(patch_thread);
    app.active_thread_id = Some(patch_thread);
    app.replay_agents_overview_requests(&command_server, patch_thread)
        .await;
    app.submit_thread_op(
        &mut patch_server,
        patch_thread,
        AppCommand::PatchApproval {
            id: "patch".into(),
            decision: FileChangeApprovalDecision::Accept,
        },
    )
    .await?;

    assert_eq!(
        timeout(Duration::from_secs(/*secs*/ 5), command_replies.recv()).await?,
        Some(JSONRPCMessage::Response(JSONRPCResponse {
            id: AppServerRequestId::Integer(7),
            result: json!({"decision":"decline"})
        }))
    );
    assert_eq!(
        timeout(Duration::from_secs(/*secs*/ 5), patch_replies.recv()).await?,
        Some(JSONRPCMessage::Response(JSONRPCResponse {
            id: AppServerRequestId::Integer(7),
            result: json!({"decision":"accept"})
        }))
    );
    command_server.shutdown().await?;
    patch_server.shutdown().await?;
    command_task.await??;
    patch_task.await??;
    Ok(())
}

#[tokio::test]
async fn closed_origin_consumes_reply_without_falling_through() -> color_eyre::Result<()> {
    let (mut app, _events, _ops) = crate::app::tests::make_test_app_with_channels().await;
    let (original, _replies, task) = connection().await?;
    let thread = ThreadId::new();
    let request = BufferedServerRequest::new(patch_request(thread), &original.request_handle());
    app.pending_app_server_requests
        .note_buffered_request(&request);
    app.enqueue_thread_request(thread, request.request).await?;
    original.shutdown().await?;
    task.await??;

    assert!(
        app.try_resolve_app_server_request(
            thread,
            &AppCommand::PatchApproval {
                id: "patch".into(),
                decision: FileChangeApprovalDecision::Accept,
            },
        )
        .await?
    );
    assert!(
        app.thread_event_channels[&thread]
            .store
            .lock()
            .await
            .pending_replay_requests()
            .is_empty()
    );
    Ok(())
}
