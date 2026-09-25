//! Detect configured tunnel protocols without consuming bytes needed for opaque forwarding.
//! HTTP requests retain their authorized tunnel destination; upgraded streams stay opaque.

use anyhow::Context;
use anyhow::Result;
use rama_core::Service;
use rama_core::extensions::ExtensionsMut;
use rama_core::extensions::ExtensionsRef;
use rama_core::stream::PeekStream;
use rama_core::stream::Stream;
use rama_http::HeaderValue;
use rama_http::Method;
use rama_http::Request;
use rama_http::Response;
use rama_http::StatusCode;
use rama_http::Uri;
use rama_http::header::CONNECTION;
use rama_http::header::PROXY_AUTHENTICATE;
use rama_http::header::UPGRADE;
use rama_http::io::upgrade::OnUpgrade;
use std::io::Cursor;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::time::timeout;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct BrokeredProtocols {
    pub(crate) tls: bool,
    pub(crate) http: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TunnelProtocol {
    Tls,
    Http,
    Opaque,
}

const PREFIX_TIMEOUT: Duration = Duration::from_millis(250);
const MAX_REQUEST_LINE: usize = 8192;

/// Detect only configured protocols, replaying every byte for opaque forwarding.
pub(crate) async fn peek_protocol<S>(
    mut stream: S,
    protocols: BrokeredProtocols,
) -> Result<(TunnelProtocol, PeekStream<Cursor<Vec<u8>>, S>)>
where
    S: Stream + Unpin + ExtensionsMut,
{
    let mut prefix = Vec::new();
    let mut buf = [0_u8; 5];
    if let Ok(result) = timeout(PREFIX_TIMEOUT, stream.read(&mut buf)).await {
        prefix.extend_from_slice(&buf[..result.context("read tunnel prefix")?])
    }
    let protocol = if prefix.first() == Some(&0x16) && protocols.tls {
        // Do not time out a fragmented TLS record once the client has started its handshake.
        while prefix.len() < 5
            && matches!(
                prefix.as_slice(),
                [0x16] | [0x16, 0x03] | [0x16, 0x03, 0x00..=0x04, ..]
            )
        {
            let read = stream.read(&mut buf[..5 - prefix.len()]).await?;
            if read == 0 {
                break;
            }
            prefix.extend_from_slice(&buf[..read]);
        }
        if matches!(prefix.as_slice(), [0x16, 0x03, 0x00..=0x04, _, _]) {
            TunnelProtocol::Tls
        } else {
            TunnelProtocol::Opaque
        }
    } else if protocols.http && !prefix.is_empty() {
        // A bounded probe avoids stalling client-first opaque protocols that wait for a reply.
        let probe = async {
            let mut buf = [0_u8; 1024];
            loop {
                if let Some(end) = prefix.iter().position(|byte| *byte == b'\n') {
                    let line = std::str::from_utf8(&prefix[..=end]).ok()?;
                    let mut parts = line.strip_suffix("\r\n")?.split(' ');
                    let method = parts.next()?;
                    let target = parts.next()?;
                    let version = parts.next()?;
                    return (parts.next().is_none()
                        && Method::from_bytes(method.as_bytes()).is_ok()
                        && target.parse::<Uri>().is_ok()
                        && (matches!(version, "HTTP/1.0" | "HTTP/1.1")
                            || (method == "PRI" && target == "*" && version == "HTTP/2.0")))
                        .then_some(());
                }
                if prefix.len() >= MAX_REQUEST_LINE
                    || prefix
                        .iter()
                        .any(|byte| !matches!(byte, b'\r' | b'\t' | 0x20..=0x7e))
                {
                    return None;
                }
                let remaining = buf.len().min(MAX_REQUEST_LINE - prefix.len());
                let read = stream.read(&mut buf[..remaining]).await.ok()?;
                if read == 0 {
                    return None;
                }
                prefix.extend_from_slice(&buf[..read]);
            }
        };
        if matches!(timeout(PREFIX_TIMEOUT, probe).await, Ok(Some(()))) {
            TunnelProtocol::Http
        } else {
            TunnelProtocol::Opaque
        }
    } else {
        TunnelProtocol::Opaque
    };
    Ok((protocol, PeekStream::new(Cursor::new(prefix), stream)))
}

/// Preserve HTTP/1 upgrades on an already-authorized tunnel without interpreting upgraded bytes.
pub(crate) async fn forward_http_request(
    mut req: Request,
    upstream: &crate::upstream::UpstreamClient,
) -> Result<Response> {
    let upgrade = req.headers().get(UPGRADE).cloned().filter(|_| {
        req.headers().get_all(CONNECTION).iter().any(|value| {
            value.to_str().is_ok_and(|value| {
                value
                    .split(',')
                    .any(|token| token.trim().eq_ignore_ascii_case("upgrade"))
            })
        })
    });
    let downstream_upgrade = req.extensions().get::<OnUpgrade>().cloned();
    let h2c_settings = if upgrade.as_ref().is_some_and(|value| {
        value.to_str().is_ok_and(|value| {
            value
                .split(',')
                .any(|protocol| protocol.trim().eq_ignore_ascii_case("h2c"))
        })
    }) {
        req.headers()
            .get_all("http2-settings")
            .iter()
            .cloned()
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    crate::http_proxy::remove_hop_by_hop_request_headers(req.headers_mut());
    if let Some(upgrade) = &upgrade {
        req.headers_mut().insert(UPGRADE, upgrade.clone());
        // The upgraded stream is relayed unchanged, including the client's h2c negotiation.
        let connection = if h2c_settings.is_empty() {
            "upgrade"
        } else {
            "upgrade, http2-settings"
        };
        if !h2c_settings.is_empty() {
            req.headers_mut().remove("http2-settings");
        }
        for settings in h2c_settings {
            req.headers_mut().append("http2-settings", settings);
        }
        req.headers_mut()
            .insert(CONNECTION, HeaderValue::from_static(connection));
    }
    let mut response = upstream.serve(req).await?;
    let response_upgrade = if response.status() == StatusCode::SWITCHING_PROTOCOLS {
        let expected = upgrade
            .as_ref()
            .context("unsolicited HTTP tunnel upgrade")?
            .to_str()?;
        let selected = response
            .headers()
            .get(UPGRADE)
            .context("missing HTTP upgrade protocol")?
            .clone();
        anyhow::ensure!(
            expected.split(',').any(|protocol| protocol
                .trim()
                .eq_ignore_ascii_case(selected.to_str().unwrap_or_default())),
            "HTTP tunnel upgrade protocol mismatch"
        );
        let downstream = downstream_upgrade.context("missing downstream HTTP upgrade")?;
        let upstream = response
            .extensions()
            .get::<OnUpgrade>()
            .cloned()
            .context("missing upstream HTTP upgrade")?;
        tokio::spawn(async move {
            let result = async {
                let (mut downstream, mut upstream) = tokio::try_join!(downstream, upstream)?;
                tokio::io::copy_bidirectional(&mut downstream, &mut upstream).await?;
                anyhow::Ok(())
            }
            .await;
            if let Err(err) = result {
                tracing::warn!("HTTP tunnel upgrade failed: {err}");
            }
        });
        Some(selected)
    } else {
        None
    };
    crate::http_proxy::remove_hop_by_hop_request_headers(response.headers_mut());
    response.headers_mut().remove(PROXY_AUTHENTICATE);
    if let Some(upgrade) = response_upgrade {
        response.headers_mut().insert(UPGRADE, upgrade);
        response
            .headers_mut()
            .insert(CONNECTION, HeaderValue::from_static("upgrade"));
    }
    Ok(response)
}

#[cfg(test)]
#[path = "brokered_tunnel_tests.rs"]
mod tests;
