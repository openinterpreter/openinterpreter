//! Cancellation and caller-owned identity checks for queued native operations.

use crate::UserVerificationCancellationReason;
use crate::UserVerificationError;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

/// Invalidates queued work and suppresses late results. Native providers observe cancellation
/// during authentication and request dismissal of their active OS prompt.
#[derive(Clone, Default)]
pub struct UserVerificationRequestGuard {
    cancelled: Arc<AtomicBool>,
    activity_check: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
}

impl UserVerificationRequestGuard {
    /// The callback checks a captured identity; it must be nonblocking and must not prompt.
    pub fn with_activity_check(activity_check: impl Fn() -> bool + Send + Sync + 'static) -> Self {
        Self {
            cancelled: Arc::default(),
            activity_check: Some(Arc::new(activity_check)),
        }
    }

    pub fn cancel(&self) {
        self.cancelled.store(/*val*/ true, Ordering::Release);
    }

    pub fn is_active(&self) -> bool {
        if self.activity_check.as_ref().is_some_and(|check| !check()) {
            self.cancel();
        }
        !self.cancelled.load(Ordering::Acquire)
    }

    pub fn check(&self) -> Result<(), UserVerificationError> {
        if self.is_active() {
            Ok(())
        } else {
            Err(UserVerificationError::Cancelled {
                reason: UserVerificationCancellationReason::Interrupted,
                message: "the verification operation is no longer active".to_string(),
            })
        }
    }
}

#[cfg(test)]
#[path = "guard_tests.rs"]
mod tests;
