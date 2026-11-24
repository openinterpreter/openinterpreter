//! Retained reply capability: a server request ID and the worker that emitted it.
use crate::AppServerRequestHandle;
use codex_app_server_protocol::JSONRPCErrorError;
use codex_app_server_protocol::RequestId;
use codex_app_server_protocol::Result as JsonRpcResult;
use std::fmt;
use std::io;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

/// A reply always targets its originating connection, regardless of later UI selection.
/// Clones share a single-use completion state, so only the first resolve or reject is
/// submitted. Dropping a handle sends nothing. It does not keep a shutdown worker alive
/// or reconnect it; a stale connection returns its normal I/O error. No retries occur.
#[derive(Clone)]
pub struct AppServerReplyHandle {
    client: AppServerRequestHandle,
    request_id: RequestId,
    claimed: Arc<AtomicBool>,
}
impl fmt::Debug for AppServerReplyHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AppServerReplyHandle([redacted])")
    }
}
impl AppServerRequestHandle {
    /// Capture at event ingress, using an ID from this connection's event stream.
    /// Request IDs may be reused by other connections and must not select a client.
    pub fn server_request_reply_handle(&self, request_id: RequestId) -> AppServerReplyHandle {
        AppServerReplyHandle {
            client: self.clone(),
            request_id,
            claimed: Arc::new(AtomicBool::new(false)),
        }
    }
}
impl AppServerReplyHandle {
    fn claim(&self) -> io::Result<()> {
        self.claimed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| ())
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "app-server reply handle already used",
                )
            })
    }
    /// Resolve at the caller's explicit request. Failure can have an unknown outcome.
    pub async fn resolve(&self, result: JsonRpcResult) -> io::Result<()> {
        self.claim()?;
        self.client
            .resolve_server_request(self.request_id.clone(), result)
            .await
    }
    /// Reject on the originating connection; never fall back to another connection.
    pub async fn reject(&self, error: JSONRPCErrorError) -> io::Result<()> {
        self.claim()?;
        self.client
            .reject_server_request(self.request_id.clone(), error)
            .await
    }
}

#[cfg(test)]
#[path = "server_request_tests.rs"]
mod tests;
