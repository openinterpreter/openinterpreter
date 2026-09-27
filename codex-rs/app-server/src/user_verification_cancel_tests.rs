//! Checks that client RPC cancellation reaches native work without owning an elicitation.

use super::test_support::Harness;
use super::*;
use crate::outgoing_message::ConnectionId;
use crate::outgoing_message::ConnectionRequestId;
use crate::outgoing_message::OutgoingEnvelope;
use crate::outgoing_message::OutgoingMessage;
use anyhow::Result;
use pretty_assertions::assert_eq;
use serde_json::json;
use std::sync::atomic::Ordering;

fn assert_cancelled(message: OutgoingMessage) {
    let OutgoingMessage::Error(error) = message else {
        panic!("canceled verification must not return a proof")
    };
    assert_eq!(error.id, rpc::RequestId::Integer(1));
    assert_eq!(
        error.error,
        rpc::JSONRPCErrorError {
            code: -32603,
            message: "User verification was cancelled.".into(),
            data: Some(json!({"type": "cancelled", "reason": "interrupted"})),
        }
    );
}

#[tokio::test]
async fn user_verification_cancel_rpc_stops_native_work_and_allows_another_operation() -> Result<()>
{
    let mut h = Harness::new(ConnectionOrigin::Stdio, || true).await?;
    h.initialize("Codex Desktop", /*opt_in*/ true).await;
    h.start_verify().await?;
    h.send(
        /*id*/ 2,
        "userVerification/cancel",
        json!({"requestId": 1}),
    )
    .await;
    for _ in 0..2 {
        let message = h.response().await;
        if let OutgoingMessage::Response(response) = message {
            assert_eq!(response.id, rpc::RequestId::Integer(2));
            assert_eq!(serde_json::to_value(response.result)?, json!({}));
        } else {
            assert_cancelled(message);
        }
    }
    assert_eq!(h.service.worker.available_permits(), 1);
    // Repeated and already-finished cancellation must not cancel the next operation.
    h.send(
        /*id*/ 3,
        "userVerification/cancel",
        json!({"requestId": 1}),
    )
    .await;
    assert!(matches!(h.response().await, OutgoingMessage::Response(_)));
    h.send(/*id*/ 4, "userVerification/status", json!({})).await;
    let OutgoingMessage::Response(response) = h.response().await else {
        panic!("the next native operation must remain available")
    };
    assert_eq!(response.id, rpc::RequestId::Integer(4));
    h.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn user_verification_cancel_is_registered_before_native_dispatch() -> Result<()> {
    let mut h = Harness::new(ConnectionOrigin::Stdio, || true).await?;
    h.initialize("Codex Desktop", /*opt_in*/ true).await;
    h.send(
        /*id*/ 1,
        "userVerification/verify",
        json!({"challenge": "AQ", "title": "Approve", "description": ""}),
    )
    .await;
    // This current-thread executor has not polled the spawned RPC handler yet.
    h.outgoing
        .cancel_user_verification_request(&ConnectionRequestId {
            connection_id: ConnectionId(1),
            request_id: rpc::RequestId::Integer(1),
        })
        .await;
    assert_cancelled(h.response().await);
    assert_eq!(h.provider.calls.load(Ordering::SeqCst), 0);
    h.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn user_verification_cancel_rpc_cannot_target_another_connection_or_id_type() -> Result<()> {
    let mut h = Harness::new(ConnectionOrigin::Stdio, || true).await?;
    h.initialize("Codex Desktop", /*opt_in*/ true).await;
    h.start_verify().await?;
    h.send(
        /*id*/ 2,
        "userVerification/cancel",
        json!({"requestId": "1"}),
    )
    .await;
    assert!(matches!(h.response().await, OutgoingMessage::Response(_)));
    h.processor
        .process_request(
            ConnectionId(2),
            rpc::JSONRPCRequest {
                id: rpc::RequestId::Integer(3),
                method: "userVerification/cancel".into(),
                params: Some(json!({"requestId": 1})),
                trace: None,
            },
            &crate::transport::AppServerTransport::Stdio,
            Arc::clone(&h.session),
        )
        .await;
    let OutgoingEnvelope::ToConnection {
        connection_id,
        message: OutgoingMessage::Response(response),
        ..
    } = tokio::time::timeout(Duration::from_secs(/*secs*/ 5), h.messages.recv())
        .await?
        .unwrap()
    else {
        panic!("another connection must only receive its own cancel acknowledgment")
    };
    assert_eq!(connection_id, ConnectionId(2));
    assert_eq!(response.id, rpc::RequestId::Integer(3));
    assert_eq!(h.service.worker.available_permits(), 0);
    h.outgoing
        .cancel_user_verification_request(&ConnectionRequestId {
            connection_id: ConnectionId(1),
            request_id: rpc::RequestId::Integer(1),
        })
        .await;
    assert_cancelled(h.response().await);
    h.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn user_verification_cancel_keeps_the_worker_slot_until_native_exit() -> Result<()> {
    let mut h = Harness::new(ConnectionOrigin::Stdio, || true).await?;
    h.initialize("Codex Desktop", /*opt_in*/ true).await;
    h.provider
        .ignore_cancellation
        .store(/*val*/ true, Ordering::Release);
    h.start_verify().await?;
    h.send(
        /*id*/ 2,
        "userVerification/cancel",
        json!({"requestId": 1}),
    )
    .await;
    let OutgoingMessage::Response(response) = h.response().await else {
        panic!("cancel acknowledgment must not wait for the native worker")
    };
    assert_eq!(response.id, rpc::RequestId::Integer(2));
    assert_eq!(h.service.worker.available_permits(), 0);
    h.send(/*id*/ 3, "userVerification/status", json!({})).await;
    let OutgoingMessage::Error(error) = h.response().await else {
        panic!("the native worker must remain bounded")
    };
    assert_eq!(
        error.error.data,
        Some(json!({"type": "failed", "reason": "providerError"}))
    );
    h.provider.released.store(/*val*/ true, Ordering::Release);
    assert_cancelled(h.response().await);
    assert_eq!(h.service.worker.available_permits(), 1);
    h.shutdown().await;
    Ok(())
}
