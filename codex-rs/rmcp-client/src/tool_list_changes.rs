//! Records catalog invalidations in transport order, before a later tool response can
//! let inference continue. The SDK still receives and handles every message normally.

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use rmcp::model::JsonRpcMessage;
use rmcp::model::ServerNotification;
use rmcp::service::RoleClient;
use rmcp::service::RxJsonRpcMessage;
use rmcp::service::TxJsonRpcMessage;
use rmcp::transport::IntoTransport;
use rmcp::transport::Transport;

/// Read-only observation of notifications received by one connection. Retaining this
/// handle does not keep the connection alive.
#[derive(Clone)]
pub struct ToolListRevision(pub(crate) Arc<AtomicU64>);

impl ToolListRevision {
    pub fn current(&self) -> u64 {
        self.0.load(Ordering::Acquire)
    }
}

pub(crate) fn track_tool_list_changes<T, E, A>(
    transport: T,
    revision: Arc<AtomicU64>,
) -> impl Transport<RoleClient, Error = E> + 'static
where
    T: IntoTransport<RoleClient, E, A>,
    E: std::error::Error + Send + Sync + 'static,
{
    ToolListChanges {
        inner: transport.into_transport(),
        revision,
    }
}

struct ToolListChanges<T> {
    inner: T,
    revision: Arc<AtomicU64>,
}

impl<T: Transport<RoleClient> + 'static> Transport<RoleClient> for ToolListChanges<T> {
    type Error = T::Error;

    fn send(
        &mut self,
        message: TxJsonRpcMessage<RoleClient>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static {
        self.inner.send(message)
    }

    async fn receive(&mut self) -> Option<RxJsonRpcMessage<RoleClient>> {
        let message = self.inner.receive().await?;
        if let JsonRpcMessage::Notification(notification) = &message
            && matches!(
                notification.notification,
                ServerNotification::ToolListChangedNotification(_)
            )
        {
            self.revision.fetch_add(1, Ordering::AcqRel);
        }
        Some(message)
    }

    fn close(&mut self) -> impl Future<Output = Result<(), Self::Error>> + Send {
        self.inner.close()
    }
}
