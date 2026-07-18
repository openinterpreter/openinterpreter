use chrono::Local;
use chrono::TimeZone;

use crate::context::ContextualUserFragment;
use crate::context::KimiCronFire;

use super::CronTask;
use super::KimiCronService;
use super::delivery_for_task;

#[test]
fn renders_the_expected_same_thread_envelope() {
    let task = CronTask {
        id: "3957c2ae".to_string(),
        cron: "* * * * *".to_string(),
        prompt: "KIMI_CRON_SAME_THREAD_PROOF".to_string(),
        recurring: false,
        created_at: 0,
        last_fired_at: None,
    };

    assert_eq!(
        KimiCronFire::new(
            &task.id,
            &task.cron,
            task.recurring,
            /*coalesced_count*/ 1,
            /*stale*/ false,
            &task.prompt,
        )
        .render(),
        "<cron-fire jobId=\"3957c2ae\" cron=\"* * * * *\" recurring=\"false\" coalescedCount=\"1\" stale=\"false\">\n<prompt>\nKIMI_CRON_SAME_THREAD_PROOF\n</prompt>\n</cron-fire>"
    );
}

#[test]
fn one_shot_delivery_is_due_at_the_next_minute() {
    let created_at = Local
        .with_ymd_and_hms(2026, 7, 17, 14, 50, 12)
        .single()
        .expect("unambiguous local timestamp")
        .timestamp_millis();
    let due_at = Local
        .with_ymd_and_hms(2026, 7, 17, 14, 51, 0)
        .single()
        .expect("unambiguous local timestamp")
        .timestamp_millis();
    let task = CronTask {
        id: "3957c2ae".to_string(),
        cron: "* * * * *".to_string(),
        prompt: "proof".to_string(),
        recurring: false,
        created_at,
        last_fired_at: None,
    };

    let delivery = delivery_for_task(&task, due_at).expect("delivery should be due");
    assert_eq!(delivery.coalesced_count, 1);
    assert!(!delivery.stale);
}

#[tokio::test]
async fn create_list_delete_and_resume_use_stable_shapes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let service = KimiCronService::new(
        temp.path().join("cron"),
        std::sync::Arc::new(crate::current_time::SystemTimeProvider),
        codex_protocol::ThreadId::new(),
    );

    assert_eq!(
        service.list().await,
        "cron_jobs: 0\nNo cron jobs scheduled."
    );
    assert_eq!(
        service.delete("deadbeef").await,
        Err("No cron job with id deadbeef.".to_string())
    );

    let created = service
        .create(
            "*/5 * * * *",
            "KIMI_CODE_CRON_CHECK".to_string(),
            /*recurring*/ false,
        )
        .await
        .expect("cron should be created");
    let id = created
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("id: "))
        .expect("create output should contain id");
    assert_eq!(id.len(), 8);
    assert!(created.contains("cron: */5 * * * *"));
    assert!(created.contains("humanSchedule: every 5 minutes"));
    assert!(created.contains("recurring: false"));

    let listed = service.list().await;
    assert!(listed.contains("cron_jobs: 1"));
    assert!(listed.contains(&format!("id: {id}")));
    assert!(listed.contains("prompt: \"KIMI_CODE_CRON_CHECK\""));

    let resumed = KimiCronService::new(
        temp.path().join("cron"),
        std::sync::Arc::new(crate::current_time::SystemTimeProvider),
        codex_protocol::ThreadId::new(),
    );
    resumed.load_from_disk().await;
    assert!(resumed.list().await.contains(&format!("id: {id}")));

    assert_eq!(
        service.delete(id).await,
        Ok(format!("Deleted cron job {id}."))
    );
}
