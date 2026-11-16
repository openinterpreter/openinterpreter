use std::io;
use std::net::Ipv4Addr;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;

use pretty_assertions::assert_eq;
use reqwest::dns::Resolve;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::net::TcpStream;

use super::FETCH_URL_NON_PUBLIC;
use super::FETCH_URL_SCHEME;
use super::FETCH_URL_UNVERIFIED;
use super::MAX_FETCH_BODY_BYTES;
use super::PinnedDns;
use super::PublicHttpTarget;
use super::extract_page_text;
use super::fetch_client_builder;
use super::read_fetch_body;
use super::resolve_public_http_url_with_lookup;
use crate::function_tool::FunctionCallError;

fn parse_url(url: &str) -> reqwest::Url {
    reqwest::Url::parse(url).unwrap_or_else(|err| panic!("test URL should parse ({url}): {err}"))
}

async fn lookup_should_not_run(host: String, _port: u16) -> io::Result<Vec<SocketAddr>> {
    panic!("DNS lookup should not run for {host}")
}

async fn lookup_private(_host: String, port: u16) -> io::Result<Vec<SocketAddr>> {
    Ok(vec![SocketAddr::from(([10, 0, 0, 1], port))])
}

async fn lookup_public(_host: String, port: u16) -> io::Result<Vec<SocketAddr>> {
    Ok(vec![SocketAddr::from(([93, 184, 216, 34], port))])
}

async fn lookup_mixed_public_and_loopback(_host: String, port: u16) -> io::Result<Vec<SocketAddr>> {
    Ok(vec![
        SocketAddr::from(([93, 184, 216, 34], port)),
        SocketAddr::from(([127, 0, 0, 1], port)),
    ])
}

async fn lookup_empty(_host: String, _port: u16) -> io::Result<Vec<SocketAddr>> {
    Ok(Vec::new())
}

async fn lookup_failed(_host: String, _port: u16) -> io::Result<Vec<SocketAddr>> {
    Err(io::Error::other("lookup failed"))
}

fn public_literal_target() -> PublicHttpTarget {
    PublicHttpTarget { pinned_dns: None }
}

#[tokio::test]
async fn rejects_non_public_ip_literals_loopback_rfc1918_link_local_and_unsafe_schemes() {
    let cases = [
        "http://127.0.0.1/",
        "https://127.0.0.1/path",
        "http://10.0.0.1/",
        "http://10.255.255.255/",
        "http://172.16.0.1/",
        "http://172.31.255.1/",
        "http://192.168.1.1/",
        "http://192.168.0.1:8080/internal",
        "http://169.254.169.254/",
        "http://169.254.1.1/",
        "http://100.64.0.1/",
        "http://0.0.0.0/",
        "http://[::1]/",
        "http://[::ffff:127.0.0.1]/",
        "http://[::ffff:10.0.0.1]/",
        "http://[fe80::1]/",
        "http://[fc00::1]/",
        "http://localhost/",
        "https://LOCALHOST/",
        "http://localhost./",
        "http://foo.localhost/",
        "file:///etc/passwd",
        "ftp://example.com/",
        "gopher://example.com/",
        "data:text/plain,hello",
        "javascript:alert(1)",
        "ws://example.com/",
        "unix:///tmp/socket",
    ];

    for url in cases {
        let parsed = parse_url(url);
        let result = resolve_public_http_url_with_lookup(&parsed, lookup_should_not_run).await;
        assert!(result.is_err(), "{url} should be rejected, got {result:?}");
    }
}

#[tokio::test]
async fn rejects_metadata_style_link_local_redirect_target() {
    let url = parse_url("http://169.254.169.254/latest/meta-data/");
    assert_eq!(
        resolve_public_http_url_with_lookup(&url, lookup_should_not_run).await,
        Err(FunctionCallError::RespondToModel(
            FETCH_URL_NON_PUBLIC.to_string()
        ))
    );
}

#[tokio::test]
async fn rejects_hostname_that_resolves_to_a_private_or_loopback_address() {
    let url = parse_url("https://example.test/docs");
    assert_eq!(
        resolve_public_http_url_with_lookup(&url, lookup_private).await,
        Err(FunctionCallError::RespondToModel(
            FETCH_URL_NON_PUBLIC.to_string()
        ))
    );

    let url = parse_url("http://example.test/");
    assert_eq!(
        resolve_public_http_url_with_lookup(&url, lookup_mixed_public_and_loopback).await,
        Err(FunctionCallError::RespondToModel(
            FETCH_URL_NON_PUBLIC.to_string()
        ))
    );
}

#[tokio::test]
async fn rejects_when_dns_cannot_prove_the_host_is_public() {
    let url = parse_url("https://example.test/");
    assert_eq!(
        resolve_public_http_url_with_lookup(&url, lookup_failed).await,
        Err(FunctionCallError::RespondToModel(
            FETCH_URL_UNVERIFIED.to_string()
        ))
    );

    assert_eq!(
        resolve_public_http_url_with_lookup(&url, lookup_empty).await,
        Err(FunctionCallError::RespondToModel(
            FETCH_URL_UNVERIFIED.to_string()
        ))
    );
}

#[tokio::test]
async fn rejects_non_http_schemes_with_the_public_contract_message() {
    let url = parse_url("file://example.com/tmp");
    assert_eq!(
        resolve_public_http_url_with_lookup(&url, lookup_should_not_run).await,
        Err(FunctionCallError::RespondToModel(
            FETCH_URL_SCHEME.to_string()
        ))
    );
}

#[tokio::test]
async fn allows_public_ip_literals_and_hostnames_that_resolve_publicly() {
    let url = parse_url("https://8.8.8.8/");
    assert_eq!(
        resolve_public_http_url_with_lookup(&url, lookup_should_not_run).await,
        Ok(public_literal_target())
    );

    let url = parse_url("http://1.1.1.1/path");
    assert_eq!(
        resolve_public_http_url_with_lookup(&url, lookup_should_not_run).await,
        Ok(public_literal_target())
    );

    let url = parse_url("http://172.15.0.1/");
    assert_eq!(
        resolve_public_http_url_with_lookup(&url, lookup_should_not_run).await,
        Ok(public_literal_target())
    );

    let url = parse_url("https://[2001:4860:4860::8888]/");
    assert_eq!(
        resolve_public_http_url_with_lookup(&url, lookup_should_not_run).await,
        Ok(public_literal_target())
    );

    let url = parse_url("https://example.test/path");
    assert_eq!(
        resolve_public_http_url_with_lookup(&url, lookup_public).await,
        Ok(PublicHttpTarget {
            pinned_dns: Some(PinnedDns {
                host: "example.test".to_string(),
                addrs: vec![SocketAddr::from(([93, 184, 216, 34], 443))],
            }),
        })
    );
}

struct RebindingResolver {
    calls: AtomicUsize,
    addrs: Vec<SocketAddr>,
}

impl Resolve for RebindingResolver {
    fn resolve(&self, _name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let addrs = self.addrs.clone();
        Box::pin(async move { Ok(Box::new(addrs.into_iter()) as reqwest::dns::Addrs) })
    }
}

#[tokio::test]
async fn second_dns_answer_cannot_redirect_the_connection_privately() {
    let safe_hits = Arc::new(AtomicUsize::new(0));
    let secret_hits = Arc::new(AtomicUsize::new(0));
    let safe_addr = spawn_static_http_server(
        SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
        b"safe-public-origin",
        Arc::clone(&safe_hits),
    )
    .await;
    let secret_addr = spawn_static_http_server(
        SocketAddr::from((Ipv4Addr::new(127, 0, 0, 2), 0)),
        b"SECRET-PRIVATE",
        Arc::clone(&secret_hits),
    )
    .await;

    let rebinding = Arc::new(RebindingResolver {
        calls: AtomicUsize::new(0),
        addrs: vec![secret_addr],
    });
    let client = fetch_client_builder(Some(&PinnedDns {
        host: "rebind.test".to_string(),
        addrs: vec![safe_addr],
    }))
    .dns_resolver(Arc::clone(&rebinding))
    .build()
    .expect("client");

    // Omit an explicit URL port so reqwest uses the port from the resolved
    // SocketAddr. A second DNS answer that returns `secret_addr` would then
    // connect privately if pinning did not take precedence.
    let body = client
        .get("http://rebind.test/")
        .send()
        .await
        .expect("pinned request")
        .text()
        .await
        .expect("pinned body");

    assert_eq!(body, "safe-public-origin");
    assert_eq!(safe_hits.load(Ordering::SeqCst), 1);
    assert_eq!(secret_hits.load(Ordering::SeqCst), 0);
    assert_eq!(
        rebinding.calls.load(Ordering::SeqCst),
        0,
        "pinned addrs must win over a later private DNS answer"
    );
}

#[tokio::test]
async fn stops_reading_oversized_chunked_body_without_content_length() {
    let addr = spawn_unbounded_chunked_http_server().await;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("client");
    let response = client
        .get(format!("http://{addr}/"))
        .send()
        .await
        .expect("chunked response");
    assert!(
        response.content_length().is_none(),
        "regression requires a chunked response with no Content-Length, got {:?}",
        response.content_length()
    );

    let started = Instant::now();
    let body = read_fetch_body(response).await.expect("limited body");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "streaming must stop at the size limit instead of buffering the full transfer"
    );
    assert_eq!(body.len(), MAX_FETCH_BODY_BYTES);
    assert!(body.bytes().all(|byte| byte == b'A'));
}

#[tokio::test]
async fn escaped_entities_in_fetched_page_are_decoded_once() {
    let hits = Arc::new(AtomicUsize::new(0));
    let addr = spawn_static_http_server(
        SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
        b"<p>Write &amp;lt;div&amp;gt; to show a literal &lt;div&gt; tag.</p>",
        Arc::clone(&hits),
    )
    .await;
    let response = reqwest::Client::new()
        .get(format!("http://{addr}/"))
        .send()
        .await
        .expect("static response");
    let body = read_fetch_body(response).await.expect("body");

    assert_eq!(
        extract_page_text(&body),
        "Write &lt;div&gt; to show a literal <div> tag."
    );
}

async fn spawn_static_http_server(
    bind: SocketAddr,
    body: &'static [u8],
    hits: Arc<AtomicUsize>,
) -> SocketAddr {
    let listener = TcpListener::bind(bind)
        .await
        .unwrap_or_else(|err| panic!("bind {bind}: {err}"));
    let addr = listener.local_addr().expect("server addr");
    tokio::spawn(async move {
        loop {
            let Ok((socket, _)) = listener.accept().await else {
                break;
            };
            hits.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(async move {
                serve_static_http(socket, body).await;
            });
        }
    });
    addr
}

async fn serve_static_http(mut socket: TcpStream, body: &[u8]) {
    let _ = read_http_headers(&mut socket).await;
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = socket.write_all(header.as_bytes()).await;
    let _ = socket.write_all(body).await;
}

async fn spawn_unbounded_chunked_http_server() -> SocketAddr {
    let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .await
        .expect("bind chunked server");
    let addr = listener.local_addr().expect("chunked server addr");
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let _ = read_http_headers(&mut socket).await;
                let header =
                    b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
                if socket.write_all(header).await.is_err() {
                    return;
                }
                let chunk = [b'A'; 1024];
                loop {
                    if socket.write_all(b"400\r\n").await.is_err()
                        || socket.write_all(&chunk).await.is_err()
                        || socket.write_all(b"\r\n").await.is_err()
                    {
                        return;
                    }
                }
            });
        }
    });
    addr
}

async fn read_http_headers(socket: &mut TcpStream) -> io::Result<()> {
    let mut buf = vec![0u8; 4096];
    let mut collected = Vec::new();
    loop {
        let n = socket.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        collected.extend_from_slice(&buf[..n]);
        if collected.windows(4).any(|window| window == b"\r\n\r\n") || collected.len() > 16_384 {
            break;
        }
    }
    Ok(())
}
