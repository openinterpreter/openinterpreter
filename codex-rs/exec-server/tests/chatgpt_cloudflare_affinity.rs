#![cfg(unix)]

mod common;
#[path = "common/tls_proxy.rs"]
mod tls_proxy;

use std::ffi::OsString;
use std::fs;

use codex_exec_server::HttpRedirectPolicy;
use codex_exec_server::HttpRequestParams;
use codex_exec_server::HttpRequestResponse;
use codex_exec_server::InitializeParams;
use codex_exec_server_protocol::JSONRPCMessage;
use codex_exec_server_protocol::JSONRPCResponse;
use codex_exec_server_protocol::RequestId;
use common::exec_server::ExecServerHarness;
use common::exec_server::exec_server_with_env;
use pretty_assertions::assert_eq;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tempfile::TempDir;
use tls_proxy::TlsInterceptingProxy;

const CHATGPT_MCP_URL: &str = "https://chatgpt.com/backend-api/ps/mcp";
const NON_CHATGPT_MCP_URL: &str = "https://api.openai.com/backend-api/ps/mcp";

/// Exercises the same `http/request` route used by remotely executed Streamable HTTP MCP calls.
/// Each RPC uses the shared route-aware client. The first response sets `__cflb`, and the second response
/// replaces it, proving cross-client persistence through the shared cookie store.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exec_server_replays_only_chatgpt_cloudflare_cookies() -> anyhow::Result<()> {
    let proxy = TlsInterceptingProxy::start(/*expected_requests*/ 4)?;
    let temp_dir = TempDir::new()?;
    let ca_path = temp_dir.path().join("cloudflare-affinity-test-ca.pem");
    fs::write(&ca_path, &proxy.ca_cert_pem)?;
    let proxy_url = OsString::from(&proxy.url);
    let empty = OsString::new();
    let env = vec![
        (
            OsString::from("CODEX_CA_CERTIFICATE"),
            ca_path.as_os_str().to_owned(),
        ),
        (OsString::from("HTTPS_PROXY"), proxy_url.clone()),
        (OsString::from("https_proxy"), proxy_url.clone()),
        (OsString::from("ALL_PROXY"), proxy_url.clone()),
        (OsString::from("all_proxy"), proxy_url),
        (OsString::from("NO_PROXY"), empty.clone()),
        (OsString::from("no_proxy"), empty),
    ];
    let mut server = exec_server_with_env(env, &[]).await?;
    initialize_exec_server(&mut server).await?;

    let first_response = execute_http_request(&mut server, CHATGPT_MCP_URL, "first").await?;
    assert_eq!(
        (first_response.status, first_response.body.into_inner()),
        (200, b"ok".to_vec())
    );
    let first = proxy.next_request()?;
    assert_eq!(
        (
            first.connect_authority.as_str(),
            first.request_line.as_str(),
            first.headers.get("cookie"),
        ),
        ("chatgpt.com:443", "POST /backend-api/ps/mcp HTTP/1.1", None,)
    );

    let west_response = execute_http_request(&mut server, CHATGPT_MCP_URL, "west").await?;
    assert_eq!(west_response.status, 200);
    let request_with_west_affinity = proxy.next_request()?;
    assert_eq!(
        request_with_west_affinity
            .headers
            .get("cookie")
            .cloned()
            .unwrap_or_default(),
        vec!["__cflb=west".to_string()]
    );

    let central_response = execute_http_request(&mut server, CHATGPT_MCP_URL, "central").await?;
    assert_eq!(central_response.status, 200);
    let request_with_central_affinity = proxy.next_request()?;
    assert_eq!(
        (
            request_with_central_affinity.request_line.as_str(),
            request_with_central_affinity
                .headers
                .get("cookie")
                .cloned()
                .unwrap_or_default(),
        ),
        (
            "POST /backend-api/ps/mcp HTTP/1.1",
            vec!["__cflb=central".to_string()],
        )
    );
    let other_host_response =
        execute_http_request(&mut server, NON_CHATGPT_MCP_URL, "other-host").await?;
    assert_eq!(other_host_response.status, 200);
    let other_host = proxy.next_request()?;
    assert_eq!(
        (
            other_host.connect_authority.as_str(),
            other_host.request_line.as_str(),
            other_host.headers.get("cookie"),
        ),
        (
            "api.openai.com:443",
            "POST /backend-api/ps/mcp HTTP/1.1",
            None,
        )
    );

    server.shutdown().await?;
    proxy.finish()?;
    Ok(())
}

async fn initialize_exec_server(server: &mut ExecServerHarness) -> anyhow::Result<()> {
    let initialize_id = server
        .send_request(
            "initialize",
            serde_json::to_value(InitializeParams {
                client_name: "cloudflare-affinity-test".to_string(),
                resume_session_id: None,
            })?,
        )
        .await?;
    let _: Value = wait_for_response(server, initialize_id).await?;
    server
        .send_notification("initialized", serde_json::json!({}))
        .await
}

async fn execute_http_request(
    server: &mut ExecServerHarness,
    url: &str,
    request_id: &str,
) -> anyhow::Result<HttpRequestResponse> {
    let response_id = server
        .send_request(
            "http/request",
            serde_json::to_value(HttpRequestParams {
                method: "POST".to_string(),
                url: url.to_string(),
                headers: Vec::new(),
                body: None,
                timeout_ms: Some(5_000),
                redirect_policy: HttpRedirectPolicy::Follow,
                request_id: request_id.to_string(),
                stream_response: false,
            })?,
        )
        .await?;
    wait_for_response(server, response_id).await
}

async fn wait_for_response<T>(
    server: &mut ExecServerHarness,
    request_id: RequestId,
) -> anyhow::Result<T>
where
    T: DeserializeOwned,
{
    let message = server
        .wait_for_event(|event| match event {
            JSONRPCMessage::Response(JSONRPCResponse { id, .. })
            | JSONRPCMessage::Error(codex_exec_server_protocol::JSONRPCError { id, .. }) => {
                id == &request_id
            }
            _ => false,
        })
        .await?;
    match message {
        JSONRPCMessage::Response(JSONRPCResponse { result, .. }) => {
            Ok(serde_json::from_value(result)?)
        }
        JSONRPCMessage::Error(error) => {
            anyhow::bail!("exec-server returned an error for {request_id:?}: {error:?}")
        }
        _ => unreachable!("predicate only accepts responses for the requested id"),
    }
}
