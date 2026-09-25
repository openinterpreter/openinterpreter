//! Keeps the authentication context on its owner thread while a native operation blocks.

use crate::UserVerificationError;
use crate::UserVerificationFailureReason;
use crate::UserVerificationRequestGuard;
use std::sync::mpsc;
use std::time::Duration;

pub(crate) fn run_with_cancellation<T: Send>(
    guard: &UserVerificationRequestGuard,
    operation: impl FnOnce() -> Result<T, UserVerificationError> + Send,
    cancel: impl FnOnce(),
) -> Result<T, UserVerificationError> {
    guard.check()?;
    let (done_tx, done_rx) = mpsc::sync_channel(1);
    std::thread::scope(|scope| {
        let worker = std::thread::Builder::new()
            .name("user-verification-sign".to_string())
            .spawn_scoped(scope, move || {
                let _ = done_tx.send(guard.check().and_then(|()| operation()));
            })
            .map_err(|error| {
                tracing::warn!(%error, "could not start user-verification signer");
                worker_error()
            })?;
        let result = loop {
            match done_rx.recv_timeout(Duration::from_millis(/*millis*/ 50)) {
                Ok(result) => break Some(result),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if !guard.is_active() {
                        cancel();
                        break None;
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break Some(Err(worker_error())),
            }
        };
        // Join before returning so no signer outlives the context or its lifecycle lock.
        worker.join().map_err(|_| worker_error())?;
        guard.check()?;
        result.unwrap_or_else(|| Err(worker_error()))
    })
}

fn worker_error() -> UserVerificationError {
    UserVerificationError::Failed {
        reason: UserVerificationFailureReason::ProviderError,
        message: "the user-verification signer could not complete".to_string(),
    }
}

#[cfg(test)]
#[path = "native_operation_tests.rs"]
mod tests;
