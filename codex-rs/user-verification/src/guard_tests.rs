//! Checks cancellation shared across queued operations and captured-identity callbacks.

use super::*;
use pretty_assertions::assert_eq;

#[test]
fn identity_change_permanently_cancels_all_guard_clones() {
    let identity_matches = Arc::new(AtomicBool::new(/*v*/ true));
    let activity = Arc::clone(&identity_matches);
    let guard =
        UserVerificationRequestGuard::with_activity_check(move || activity.load(Ordering::Acquire));
    let queued = guard.clone();
    assert!(queued.check().is_ok());
    identity_matches.store(/*val*/ false, Ordering::Release);
    assert_eq!(
        queued.check(),
        Err(UserVerificationError::Cancelled {
            reason: UserVerificationCancellationReason::Interrupted,
            message: "the verification operation is no longer active".to_string(),
        })
    );
    identity_matches.store(/*val*/ true, Ordering::Release);
    assert!(!guard.is_active());
}

#[test]
fn cancelling_one_clone_invalidates_queued_work() {
    let guard = UserVerificationRequestGuard::default();
    let queued = guard.clone();
    guard.cancel();
    assert!(!queued.is_active());
}
