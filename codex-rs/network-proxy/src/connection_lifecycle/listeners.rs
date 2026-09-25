//! Keeps listener tasks and their connection scope alive under the same runtime owner.

use super::scope::ConnectionLifecycle;
use anyhow::Result;
use rama_core::graceful::ShutdownGuard;
use std::future::Future;
use tokio::task::JoinSet;

pub(crate) struct ProxyListeners {
    connections: ConnectionLifecycle,
    listeners: JoinSet<Result<()>>,
}

impl ProxyListeners {
    pub(crate) fn new() -> Self {
        Self {
            connections: ConnectionLifecycle::new(),
            listeners: JoinSet::new(),
        }
    }

    pub(crate) fn spawn<F, Fut>(&mut self, listener: F)
    where
        F: FnOnce(ShutdownGuard) -> Fut,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.listeners.spawn(listener(self.connections.guard()));
    }

    pub(crate) fn cancel(&mut self) {
        self.connections.cancel();
        self.listeners.abort_all();
    }

    pub(crate) async fn wait(&mut self) -> Result<()> {
        while let Some(result) = self.listeners.join_next().await {
            result??;
        }
        Ok(())
    }

    pub(crate) async fn shutdown(mut self) {
        self.cancel();
        self.listeners.shutdown().await;
        self.connections.shutdown().await;
    }
}
