//! Checks Host field cardinality before policy evaluation and upstream forwarding.

use super::http_plain_proxy;
use crate::NetworkProxy;
use crate::NetworkProxyConfig;
use crate::runtime::network_proxy_state_for_policy;
use pretty_assertions::assert_eq;
use rama_core::extensions::ExtensionsMut;
use rama_http::Body;
use rama_http::Request;
use rama_http::StatusCode;
use std::net::Ipv4Addr;
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio::sync::oneshot;
use tokio::time::Duration;
use tokio::time::timeout;

#[derive(Clone, Copy, Debug)]
enum RequestTarget {
    Origin,
    Absolute,
    Connect,
}

#[derive(Clone, Copy, Debug)]
enum HostFields {
    Single,
    Identical,
    Conflicting,
}

async fn exchange(target: RequestTarget, hosts: HostFields) -> (String, Option<String>) {
    let origin = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let origin_addr = origin.local_addr().unwrap();
    let (stop_origin, stop_requested) = oneshot::channel::<()>();
    let origin_task = tokio::spawn(async move {
        let (mut stream, _) = tokio::select! {
            biased;
            accepted = origin.accept() => accepted.unwrap(),
            _ = stop_requested => return None,
        };
        if matches!(target, RequestTarget::Connect) {
            stream.write_all(b"fixture tunnel\r\n").await.unwrap();
            return Some("CONNECT tunnel established".to_string());
        }
        let mut headers = Vec::new();
        loop {
            let byte = stream.read_u8().await.unwrap();
            headers.push(byte);
            assert!(headers.len() < 8192);
            if headers.ends_with(b"\r\n\r\n") {
                break;
            }
        }
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK")
            .await
            .unwrap();
        Some(String::from_utf8(headers).unwrap())
    });

    let mut network = NetworkProxyConfig {
        allow_local_binding: Some(true),
        allow_upstream_proxy: false,
        enable_socks5: false,
        ..NetworkProxyConfig::default()
    };
    network.set_allowed_domains(vec!["127.0.0.1".to_string()]);
    network.set_denied_domains(vec!["localhost".to_string()]);
    let state = Arc::new(network_proxy_state_for_policy(network));
    let proxy = NetworkProxy::builder()
        .state(state)
        .managed_proxy_routing(crate::ManagedProxyRouting::DedicatedListeners)
        .build()
        .await
        .unwrap();
    let handle = proxy.run().await.unwrap();
    let mut client = TcpStream::connect(proxy.http_addr()).await.unwrap();
    let first_host = format!("Host: {origin_addr}\r\n");
    let second_host = match hosts {
        HostFields::Single => String::new(),
        HostFields::Identical => format!("hOsT: {origin_addr}\r\n"),
        HostFields::Conflicting => format!("hOsT: localhost:{}\r\n", origin_addr.port()),
    };
    let request_line = match target {
        RequestTarget::Origin => "GET / HTTP/1.1".to_string(),
        RequestTarget::Absolute => format!("GET http://{origin_addr}/ HTTP/1.1"),
        RequestTarget::Connect => format!("CONNECT {origin_addr} HTTP/1.1"),
    };
    let request = format!("{request_line}\r\n{first_host}{second_host}Connection: close\r\n\r\n");
    client.write_all(request.as_bytes()).await.unwrap();
    let mut response = Vec::new();
    timeout(Duration::from_secs(5), client.read_to_end(&mut response))
        .await
        .expect("proxy should complete the response")
        .unwrap();
    drop(client);
    let _ = stop_origin.send(());
    let origin_request = timeout(Duration::from_secs(5), origin_task)
        .await
        .expect("origin should stop")
        .unwrap();
    timeout(Duration::from_secs(5), handle.shutdown())
        .await
        .expect("proxy should shut down")
        .unwrap();
    (String::from_utf8(response).unwrap(), origin_request)
}

#[tokio::test]
async fn rejects_duplicate_host_fields_before_upstream() {
    let mut observed = Vec::new();
    for target in [
        RequestTarget::Origin,
        RequestTarget::Absolute,
        RequestTarget::Connect,
    ] {
        for hosts in [HostFields::Identical, HostFields::Conflicting] {
            let (response, origin_request) = exchange(target, hosts).await;
            observed.push((
                response.lines().next().unwrap().to_string(),
                origin_request.is_some(),
            ));
        }
    }
    assert_eq!(
        observed,
        vec![("HTTP/1.1 400 Bad Request".to_string(), false); 6]
    );
}

#[tokio::test]
async fn preserves_single_host_forwarding() {
    for target in [RequestTarget::Origin, RequestTarget::Absolute] {
        let (response, origin_request) = exchange(target, HostFields::Single).await;
        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));
        let host_count = origin_request
            .as_ref()
            .unwrap()
            .lines()
            .filter(|line| line.to_ascii_lowercase().starts_with("host:"))
            .count();
        assert_eq!(host_count, 1);
    }
}

#[tokio::test]
async fn rejects_duplicate_hosts_before_unix_socket_routing() {
    let state = Arc::new(network_proxy_state_for_policy(NetworkProxyConfig::default()));
    let mut request = Request::builder()
        .uri("/")
        .header("host", "allowed.test")
        .header("host", "denied.test")
        .header("x-unix-socket", "/not-an-allowed-socket")
        .body(Body::empty())
        .unwrap();
    request.extensions_mut().insert(state);
    let response = http_plain_proxy(
        /*policy_decider*/ None, /*environment_id*/ None, request,
    )
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
