//! Exercises prompt teardown and late-result rejection without requiring biometric hardware.

use super::*;
use crate::UserVerificationCancellationReason;
use pretty_assertions::assert_eq;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

#[test]
fn cancellation_tears_down_the_prompt_on_the_context_owner_thread() {
    let guard = UserVerificationRequestGuard::default();
    let queued = guard.clone();
    let (started_tx, started_rx) = mpsc::channel();
    let (dismissed_tx, dismissed_rx) = mpsc::channel();
    let (exited_tx, exited_rx) = mpsc::channel();
    let owner = std::thread::spawn(move || {
        let owner_thread = std::thread::current().id();
        run_with_cancellation(
            &queued,
            move || {
                started_tx.send(()).expect("authentication started");
                dismissed_rx
                    .recv_timeout(Duration::from_secs(/*secs*/ 5))
                    .expect("prompt dismissed before the deadline");
                exited_tx.send(()).expect("signer exited");
                Ok("late signature")
            },
            || {
                assert_eq!(std::thread::current().id(), owner_thread);
                dismissed_tx.send(()).expect("dismiss authentication");
            },
        )
    });
    started_rx
        .recv_timeout(Duration::from_secs(/*secs*/ 5))
        .expect("signer is waiting for authentication");
    guard.cancel();
    assert_eq!(
        owner.join().expect("owner returned"),
        Err(UserVerificationError::Cancelled {
            reason: UserVerificationCancellationReason::Interrupted,
            message: "the verification operation is no longer active".to_string(),
        })
    );
    exited_rx.try_recv().expect("signer exited before return");
}

#[test]
fn completed_operation_does_not_cancel_its_context() {
    let cancelled = AtomicBool::new(/*v*/ false);
    assert_eq!(
        run_with_cancellation(
            &UserVerificationRequestGuard::default(),
            || Ok("signature"),
            || cancelled.store(/*val*/ true, Ordering::Release),
        ),
        Ok("signature")
    );
    assert!(!cancelled.load(Ordering::Acquire));
}

#[test]
fn already_cancelled_operation_never_starts_a_signer() {
    let guard = UserVerificationRequestGuard::default();
    guard.cancel();
    let started = AtomicBool::new(/*v*/ false);
    let result = run_with_cancellation(
        &guard,
        || {
            started.store(/*val*/ true, Ordering::Release);
            Ok(())
        },
        || panic!("no context needs cancellation"),
    );
    assert_eq!(
        result,
        Err(UserVerificationError::Cancelled {
            reason: UserVerificationCancellationReason::Interrupted,
            message: "the verification operation is no longer active".to_string(),
        })
    );
    assert!(!started.load(Ordering::Acquire));
}
