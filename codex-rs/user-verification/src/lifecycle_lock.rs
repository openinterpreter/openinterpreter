//! Serializes operations for one account across local processes, with cancellable lock waits.

use crate::UserVerificationError;
use crate::UserVerificationFailureReason;
use crate::UserVerificationRequestGuard;
use std::fs;
use std::fs::File;
use std::fs::OpenOptions;
use std::path::Path;
use std::time::Duration;
use std::time::Instant;

pub(crate) struct LifecycleLock {
    _file: File,
}

impl LifecycleLock {
    #[cfg(target_os = "macos")]
    pub(crate) fn acquire(
        namespace: &crate::UserVerificationKeyNamespace,
        guard: &UserVerificationRequestGuard,
    ) -> Result<Self, UserVerificationError> {
        let directory = dirs::data_local_dir().ok_or_else(|| UserVerificationError::Failed {
            reason: UserVerificationFailureReason::ProviderError,
            message: "could not locate the credential lock directory".to_string(),
        })?;
        Self::acquire_at(
            &directory
                .join("com.openai.codex")
                .join("user-verification")
                .join(format!("{}.lock", namespace.label)),
            Duration::from_secs(/*secs*/ 60),
            guard,
        )
    }

    fn acquire_at(
        path: &Path,
        timeout: Duration,
        guard: &UserVerificationRequestGuard,
    ) -> Result<Self, UserVerificationError> {
        guard.check()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(lock_error)?;
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(lock_error)?;
        let started = Instant::now();
        loop {
            guard.check()?;
            match file.try_lock() {
                Ok(()) => {
                    guard.check()?;
                    return Ok(Self { _file: file });
                }
                Err(std::fs::TryLockError::WouldBlock) if started.elapsed() >= timeout => {
                    return Err(UserVerificationError::Failed {
                        reason: UserVerificationFailureReason::Timeout,
                        message: "timed out waiting for another credential operation".to_string(),
                    });
                }
                Err(std::fs::TryLockError::WouldBlock) => {
                    std::thread::sleep(Duration::from_millis(/*millis*/ 50).min(timeout));
                }
                Err(std::fs::TryLockError::Error(error)) => return Err(lock_error(error)),
            }
        }
    }
}

fn lock_error(error: std::io::Error) -> UserVerificationError {
    tracing::warn!(%error, "user-verification lifecycle lock failed");
    UserVerificationError::Failed {
        reason: UserVerificationFailureReason::ProviderError,
        message: "could not acquire the credential lock".to_string(),
    }
}

#[cfg(test)]
#[path = "lifecycle_lock_tests.rs"]
mod tests;
