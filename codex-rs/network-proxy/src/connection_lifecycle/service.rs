//! Cancels connection work when its executor's proxy scope ends, including HTTP upgrades.

use rama_core::Service;
use rama_core::extensions::ExtensionsRef;
use rama_core::rt::Executor;

#[derive(Clone)]
pub(crate) struct CancelOnShutdown<S> {
    inner: S,
}

impl<S> CancelOnShutdown<S> {
    pub(crate) fn new(inner: S) -> Self {
        Self { inner }
    }
}

impl<S, Request> Service<Request> for CancelOnShutdown<S>
where
    S: Service<Request, Output = ()>,
    Request: ExtensionsRef + Send + 'static,
{
    type Output = ();
    type Error = S::Error;

    async fn serve(&self, request: Request) -> Result<(), Self::Error> {
        let guard = request
            .extensions()
            .get::<Executor>()
            .and_then(Executor::guard)
            .cloned();
        match guard {
            Some(guard) => {
                tokio::select! {
                    biased;
                    _ = guard.cancelled() => Ok(()),
                    result = self.inner.serve(request) => result,
                }
            }
            None => self.inner.serve(request).await,
        }
    }
}
