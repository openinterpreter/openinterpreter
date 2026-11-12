//! Shared recovery for idempotent V1 and V2 discovery requests.

use super::Environment;
use super::EnvironmentConnectionState;
use crate::ExecServerError;
use crate::client::is_retryable_recovery_error;
use std::future::Future;
use std::time::Duration;
use tokio::sync::watch;

impl Environment {
    /// Replays discovery once after recovery; callers subscribe before obtaining the client.
    pub(super) async fn discover_with_recovery<T, F, Fut>(
        &self,
        mut connection_state: watch::Receiver<EnvironmentConnectionState>,
        discover: F,
    ) -> Result<T, ExecServerError>
    where
        F: Fn() -> Fut,
        Fut: Future<Output = Result<T, ExecServerError>>,
    {
        match discover().await {
            Err(error) if is_retryable_recovery_error(&error) => {
                tracing::warn!(%error, "replaying capability discovery after executor recovery");
                let recovered = tokio::time::timeout(Duration::from_secs(8), async {
                    loop {
                        match self.readiness_result() {
                            Some(Ok(())) => return Some(Ok(())),
                            Some(Err(error)) if !is_retryable_recovery_error(&error) => {
                                return Some(Err(error));
                            }
                            Some(Err(_)) | None => {}
                        }
                        if connection_state.changed().await.is_err() {
                            return None;
                        }
                    }
                })
                .await;
                match recovered {
                    Ok(Some(Ok(()))) => discover().await,
                    Ok(Some(Err(error))) => Err(error),
                    Ok(None) | Err(_) => Err(error),
                }
            }
            response => response,
        }
    }
}
