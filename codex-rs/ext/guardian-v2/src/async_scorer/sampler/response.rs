//! Owns an in-flight classifier stream and its exclusive transport lease.
//! Normal completion reuses the lease. Dropped work delegates interruption and
//! bounded draining to the transport, which confirms whether the socket is reusable.
//! Cancellation before dispatch also returns the unused socket to the pool.
//! The lease bounds cleanup tasks with the same permits as active classifications.

use std::sync::Arc;
use std::time::Duration;

use codex_api::ApiError;
use codex_api::ResponseEvent;
use codex_api::ResponseInterrupt;
use codex_api::ResponseStream;
use codex_extension_api::ExtensionMetrics;
use codex_protocol::protocol::TokenUsage;

use super::connection_pool::ConnectionLease;
use super::execution::record_token_usage;

pub(super) struct PendingResponse {
    response: Option<(ResponseStream, ConnectionLease)>,
    metrics: Option<Arc<dyn ExtensionMetrics>>,
}

impl PendingResponse {
    pub(super) fn new(
        stream: ResponseStream,
        lease: ConnectionLease,
        metrics: Option<Arc<dyn ExtensionMetrics>>,
    ) -> Self {
        Self {
            response: Some((stream, lease)),
            metrics,
        }
    }

    pub(super) async fn next(&mut self) -> Option<Result<ResponseEvent, ApiError>> {
        self.response.as_mut()?.0.rx_event.recv().await
    }

    pub(super) async fn reuse(mut self, token_usage: Option<&TokenUsage>) {
        if let Some((_, lease)) = self.response.take() {
            record_token_usage(self.metrics.as_deref(), token_usage);
            lease.reuse().await;
        }
    }
}

impl Drop for PendingResponse {
    fn drop(&mut self) {
        let Some((mut stream, lease)) = self.response.take() else {
            return;
        };
        let Some(interrupt) = stream.interrupt.take() else {
            // HTTP cancellation has no response-interrupt handshake.
            return;
        };
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        // Signal before cleanup is scheduled so the transport can skip undispatched work.
        // Completion may already be queued even if the interrupt receiver has closed.
        let _ = interrupt.send(ResponseInterrupt::Cancel {
            deadline: tokio::time::Instant::now() + Duration::from_secs(/*secs*/ 1),
        });
        let metrics = self.metrics.clone();
        runtime.spawn(async move {
            while let Some(Ok(event)) = stream.rx_event.recv().await {
                if let ResponseEvent::Completed { token_usage, .. } = event {
                    record_token_usage(metrics.as_deref(), token_usage.as_ref());
                    break;
                }
            }
            // The transport retires failed streams before reporting an error. Clean EOF
            // also leaves an unused socket reusable when cancellation preceded dispatch.
            lease.reuse().await;
        });
    }
}
