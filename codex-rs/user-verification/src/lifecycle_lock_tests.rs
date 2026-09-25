//! Exercises real file-lock contention and cancellation without releasing the held lock.

use super::*;
use crate::UserVerificationCancellationReason;
use pretty_assertions::assert_eq;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

#[test]
fn lock_serializes_operations_and_releases_on_drop() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("credential.lock");
    let guard = UserVerificationRequestGuard::default();
    let first = LifecycleLock::acquire_at(&path, Duration::ZERO, &guard).expect("first lock");
    assert_eq!(
        LifecycleLock::acquire_at(&path, Duration::ZERO, &guard).err(),
        Some(UserVerificationError::Failed {
            reason: UserVerificationFailureReason::Timeout,
            message: "timed out waiting for another credential operation".to_string(),
        })
    );
    drop(first);
    LifecycleLock::acquire_at(&path, Duration::ZERO, &guard).expect("lock after release");
}

#[test]
fn cancelled_waiter_stops_while_another_operation_still_holds_lock() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("credential.lock");
    let holder = LifecycleLock::acquire_at(
        &path,
        Duration::ZERO,
        &UserVerificationRequestGuard::default(),
    )
    .expect("held lock");
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let checks = AtomicUsize::new(/*v*/ 0);
    let guard = UserVerificationRequestGuard::with_activity_check(move || {
        if checks.fetch_add(/*val*/ 1, Ordering::Relaxed) == 1 {
            ready_tx.send(()).expect("notify lock wait");
        }
        true
    });
    let queued = guard.clone();
    let worker = std::thread::spawn(move || {
        LifecycleLock::acquire_at(&path, Duration::from_secs(/*secs*/ 5), &queued).err()
    });
    ready_rx
        .recv_timeout(Duration::from_secs(/*secs*/ 5))
        .expect("waiter entered lock loop");
    guard.cancel();
    assert_eq!(
        worker.join().expect("waiter stopped"),
        Some(UserVerificationError::Cancelled {
            reason: UserVerificationCancellationReason::Interrupted,
            message: "the verification operation is no longer active".to_string(),
        })
    );
    drop(holder);
}
