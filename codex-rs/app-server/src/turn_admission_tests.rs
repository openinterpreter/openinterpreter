use super::TurnAdmission;
use crate::error_code::INVALID_REQUEST_ERROR_CODE;
use codex_extension_api::TurnStartAdmission;
use pretty_assertions::assert_eq;

#[test]
fn drain_rejects_new_work_without_waiting_for_admitted_work() {
    let admission = TurnAdmission::default();
    let active = admission.subscribe_active();
    let in_flight = admission.admit().expect("initial request admitted");
    let automatic = admission
        .admit_turn_start()
        .expect("automatic start admitted");
    admission.begin_drain();
    assert_eq!(*active.borrow(), 2);
    assert!(admission.admit_turn_start().is_none());

    let error = admission.admit().err().expect("new request rejected");
    assert_eq!(error.code, INVALID_REQUEST_ERROR_CODE);
    assert_eq!(error.data, None);
    drop(in_flight);
    assert_eq!(*active.borrow(), 1);
    drop(automatic);
    assert_eq!(*active.borrow(), 0);
}

#[tokio::test]
async fn account_pause_waits_for_existing_work_and_resumes_on_drop() {
    let admission = TurnAdmission::default();
    let in_flight = admission.admit().expect("admitted before pause");
    let owner = admission.clone();
    let pause_task = tokio::spawn(async move { owner.pause().await });
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while admission.admit().is_ok() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("pause closes admission");
    assert!(!pause_task.is_finished());
    drop(in_flight);
    let pause = pause_task
        .await
        .expect("pause task")
        .expect("exclusive pause");
    assert!(admission.admit_turn_start().is_none());
    drop(pause);
    assert!(admission.admit().is_ok());
}

#[tokio::test]
async fn cancelled_pause_restores_admission_without_reopening_drain() {
    let admission = TurnAdmission::default();
    let in_flight = admission.admit().expect("admitted before pause");
    let owner = admission.clone();
    let pause_task = tokio::spawn(async move { owner.pause().await });
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while admission.admit().is_ok() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("pause closes admission");
    pause_task.abort();
    let _ = pause_task.await;
    drop(in_flight);
    assert!(admission.admit().is_ok());
    admission.begin_drain();
    assert!(admission.pause().await.is_err());
    assert!(admission.admit().is_err());
}
