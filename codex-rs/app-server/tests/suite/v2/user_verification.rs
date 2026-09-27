use super::connection_handling_websocket::connect_websocket;
use super::connection_handling_websocket::read_error_for_id;
use super::connection_handling_websocket::read_response_for_id;
use super::connection_handling_websocket::send_request;
use super::connection_handling_websocket::spawn_websocket_server;
use anyhow::Result;
use app_test_support::DEFAULT_CLIENT_NAME;
use app_test_support::TestAppServer;
use codex_app_server_protocol::ClientInfo;
use codex_app_server_protocol::InitializeCapabilities;
use codex_app_server_protocol::JSONRPCErrorError;
use codex_app_server_protocol::RequestId;
use pretty_assertions::assert_eq;
use serde_json::json;

#[tokio::test]
async fn user_verification_methods_require_experimental_opt_in() -> Result<()> {
    let mut app_server = TestAppServer::builder().build().await?;
    app_server
        .initialize_with_capabilities(
            ClientInfo {
                name: DEFAULT_CLIENT_NAME.into(),
                title: None,
                version: "0.1.0".into(),
            },
            Some(InitializeCapabilities {
                experimental_api: false,
                ..Default::default()
            }),
        )
        .await?;

    for (method, params) in [
        ("userVerification/status", json!({})),
        ("userVerification/enroll", json!({})),
        ("userVerification/delete", json!({})),
        ("userVerification/cancel", json!({"requestId": 1})),
        (
            "userVerification/verify",
            json!({"challenge": "AQ", "title": "Approve", "description": ""}),
        ),
    ] {
        let id = app_server.send_raw_request(method, Some(params)).await?;
        let response = app_server
            .read_stream_until_error_message(RequestId::Integer(id))
            .await?;
        assert_eq!(
            response.error,
            JSONRPCErrorError {
                code: -32600,
                message: format!("{method} requires experimentalApi capability"),
                data: None,
            }
        );
    }
    Ok(())
}

#[tokio::test]
async fn user_verification_status_without_account_returns_local_readiness() -> Result<()> {
    let mut app_server = TestAppServer::builder().build_initialized().await?;
    let id = app_server
        .send_raw_request("userVerification/status", Some(json!({})))
        .await?;
    let response = app_server
        .read_stream_until_response_message(RequestId::Integer(id))
        .await?;
    assert_eq!(
        response.result,
        json!({
            "credentialId": null,
            "unavailableReason": "providerUnavailable",
            "unavailableMessage": "User verification is not available in this build or account."
        })
    );
    Ok(())
}

#[tokio::test]
async fn user_verification_cancel_stdio_validates_and_acknowledges_unknown_requests() -> Result<()>
{
    let mut app_server = TestAppServer::builder().build().await?;
    let id = app_server
        .send_raw_request(
            "userVerification/cancel",
            Some(json!({"requestId": "verify"})),
        )
        .await?;
    let error = app_server
        .read_stream_until_error_message(RequestId::Integer(id))
        .await?;
    assert_eq!(error.error.message, "Not initialized");
    app_server.initialize().await?;
    for params in [
        json!({}),
        json!({"requestId": null}),
        json!({"requestId": 1.5}),
        json!({"requestId": 1, "connectionId": 2}),
    ] {
        let id = app_server
            .send_raw_request("userVerification/cancel", Some(params))
            .await?;
        app_server
            .read_stream_until_error_message(RequestId::Integer(id))
            .await?;
    }
    for request_id in [json!(100), json!("100"), json!("100")] {
        let id = app_server
            .send_raw_request(
                "userVerification/cancel",
                Some(json!({"requestId": request_id})),
            )
            .await?;
        let response = app_server
            .read_stream_until_response_message(RequestId::Integer(id))
            .await?;
        assert_eq!(response.result, json!({}));
    }
    Ok(())
}

#[tokio::test]
async fn user_verification_without_provider_returns_typed_unavailability() -> Result<()> {
    let mut app_server = TestAppServer::builder().build_initialized().await?;
    let id = app_server
        .send_raw_request(
            "userVerification/verify",
            Some(json!({
                "challenge": "AQ", "title": "Approve", "description": ""
            })),
        )
        .await?;
    let response = app_server
        .read_stream_until_error_message(RequestId::Integer(id))
        .await?;
    assert_eq!(
        response.error,
        JSONRPCErrorError {
            code: -32603,
            message: "User verification is not available in this build or account.".into(),
            data: Some(json!({"type": "unavailable", "reason": "providerUnavailable"})),
        }
    );
    Ok(())
}

#[tokio::test]
async fn user_verification_stdio_requires_initialization_then_validates_challenges() -> Result<()> {
    let mut app_server = TestAppServer::builder().build().await?;
    let params = json!({"challenge": "!", "title": "Approve", "description": ""});
    let id = app_server
        .send_raw_request("userVerification/verify", Some(params.clone()))
        .await?;
    let response = app_server
        .read_stream_until_error_message(RequestId::Integer(id))
        .await?;
    assert_eq!(
        response.error,
        JSONRPCErrorError {
            code: -32600,
            message: "Not initialized".into(),
            data: None,
        }
    );
    app_server.initialize().await?;
    let id = app_server
        .send_raw_request("userVerification/verify", Some(params))
        .await?;
    let response = app_server
        .read_stream_until_error_message(RequestId::Integer(id))
        .await?;
    assert_eq!(
        response.error,
        JSONRPCErrorError {
            code: -32602,
            message: "Invalid verification challenge or display text.".into(),
            data: Some(json!({"type": "invalidRequest", "reason": "invalidParams"})),
        }
    );
    Ok(())
}

#[tokio::test]
async fn user_verification_websocket_blocks_native_operations_before_validation() -> Result<()> {
    // TestAppServer speaks stdio; use the existing transport fixture for real WebSocket frames.
    let home = tempfile::tempdir()?;
    let (mut process, address) = spawn_websocket_server(home.path()).await?;
    let mut websocket = connect_websocket(address).await?;
    send_request(
        &mut websocket,
        "initialize",
        /*id*/ 0,
        Some(json!({
            "clientInfo": {"name": "codex-tui", "version": "1"},
            "capabilities": {"experimentalApi": true},
        })),
    )
    .await?;
    read_response_for_id(&mut websocket, /*id*/ 0).await?;
    send_request(
        &mut websocket,
        "userVerification/status",
        /*id*/ 1,
        Some(json!({})),
    )
    .await?;
    assert_eq!(
        read_response_for_id(&mut websocket, /*id*/ 1).await?.result,
        json!({
            "credentialId": null,
            "unavailableReason": "providerUnavailable",
            "unavailableMessage": "User verification is not available in this build or account."
        })
    );

    // This challenge produces invalidRequest over stdio. A network peer must be
    // rejected before validation, regardless of account or biometric hardware.
    for (method, params) in [
        ("userVerification/enroll", json!({})),
        ("userVerification/delete", json!({})),
        (
            "userVerification/verify",
            json!({"challenge": "!", "title": "Approve", "description": ""}),
        ),
    ] {
        send_request(&mut websocket, method, /*id*/ 2, Some(params)).await?;
        assert_eq!(
            read_error_for_id(&mut websocket, /*id*/ 2).await?.error,
            JSONRPCErrorError {
                code: -32603,
                message: "User verification is not available in this build or account.".into(),
                data: Some(json!({"type": "unavailable", "reason": "providerUnavailable"})),
            }
        );
    }
    process.kill().await?;
    Ok(())
}
