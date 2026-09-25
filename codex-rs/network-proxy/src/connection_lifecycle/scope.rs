//! Owns cancellation and completion of a proxy's accepted connections and descendant tasks.

use rama_core::graceful::Shutdown;
use rama_core::graceful::ShutdownGuard;
use tokio::sync::oneshot;

pub(crate) struct ConnectionLifecycle {
    // Only the runtime owner holds the sender. Dropping it also signals shutdown when an
    // enclosing wait/shutdown future is cancelled, without spawning another cleanup task.
    cancel: Option<oneshot::Sender<()>>,
    shutdown: Shutdown,
}

impl ConnectionLifecycle {
    pub(crate) fn new() -> Self {
        let (cancel, signal) = oneshot::channel();
        Self {
            cancel: Some(cancel),
            shutdown: Shutdown::new(signal),
        }
    }

    pub(crate) fn guard(&self) -> ShutdownGuard {
        self.shutdown.guard()
    }

    pub(crate) fn cancel(&mut self) {
        self.cancel.take();
    }

    pub(crate) async fn shutdown(mut self) {
        self.cancel();
        self.shutdown.shutdown().await;
    }
}
