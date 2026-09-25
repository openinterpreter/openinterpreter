use super::*;
use crate::SqliteConfig;
use crate::Stage1JobClaimOutcome;
use crate::runtime::test_support::test_thread_metadata;
use codex_utils_absolute_path::test_support::PathExt;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn versions_isolate_jobs_outputs_and_reset_without_losing_threads() -> anyhow::Result<()> {
    let home = crate::runtime::test_support::unique_temp_dir();
    let sqlite = SqliteConfig::new_for_testing(home.as_path().abs());
    let db = StateRuntime::init(sqlite.clone(), "test-provider".to_string()).await?;
    assert!(!sqlite.memories_v2_db_path().exists());
    let thread_id = ThreadId::new();
    let metadata = test_thread_metadata(home.as_path(), thread_id, home.as_path().join("project"));
    db.upsert_thread(&metadata).await?;

    for version in [MemoryVersion::V1, MemoryVersion::V2] {
        let store = db.memories_for_version(version).await?;
        let Stage1JobClaimOutcome::Claimed { ownership_token } = store
            .try_claim_stage1_job(
                thread_id,
                thread_id,
                metadata.updated_at.timestamp(),
                /*lease_seconds*/ 60,
                /*max_running_jobs*/ 1,
            )
            .await?
        else {
            panic!("each version must independently claim the same source")
        };
        assert!(
            store
                .mark_stage1_job_succeeded(
                    thread_id,
                    &ownership_token,
                    metadata.updated_at.timestamp(),
                    "raw",
                    version.directory_name(),
                    /*rollout_slug*/ None
                )
                .await?
        );
    }
    for version in [MemoryVersion::V1, MemoryVersion::V2] {
        let outputs = db
            .memories_for_version(version)
            .await?
            .list_stage1_outputs_for_global(/*n*/ 10)
            .await?;
        assert_eq!(
            outputs
                .iter()
                .map(|output| output.rollout_summary.as_str())
                .collect::<Vec<_>>(),
            vec![version.directory_name()]
        );
    }
    // Reopening under v1 must still clear existing v2 state.
    db.close().await;
    let db = StateRuntime::init(sqlite.clone(), "test-provider".to_string()).await?;
    db.delete_thread(thread_id).await?;
    for version in [MemoryVersion::V1, MemoryVersion::V2] {
        assert!(
            db.memories_for_version(version)
                .await?
                .list_stage1_outputs_for_global(/*n*/ 10)
                .await?
                .is_empty()
        );
    }
    db.upsert_thread(&metadata).await?;
    for version in [MemoryVersion::V1, MemoryVersion::V2] {
        let store = db.memories_for_version(version).await?;
        let Stage1JobClaimOutcome::Claimed { ownership_token } = store
            .try_claim_stage1_job(
                thread_id,
                thread_id,
                metadata.updated_at.timestamp(),
                /*lease_seconds*/ 60,
                /*max_running_jobs*/ 1,
            )
            .await?
        else {
            panic!("deletion must clear the job watermark")
        };
        store
            .mark_stage1_job_succeeded(
                thread_id,
                &ownership_token,
                metadata.updated_at.timestamp(),
                "raw",
                "summary",
                /*rollout_slug*/ None,
            )
            .await?;
    }
    db.clear_all_memory_data().await?;
    for version in [MemoryVersion::V1, MemoryVersion::V2] {
        assert!(
            db.memories_for_version(version)
                .await?
                .list_stage1_outputs_for_global(/*n*/ 10)
                .await?
                .is_empty()
        );
    }
    assert!(db.get_thread(thread_id).await?.is_some());
    db.close().await;
    assert!(StateRuntime::clear_memory_data_in_sqlite_home(&sqlite).await?);
    tokio::fs::remove_dir_all(home).await?;
    Ok(())
}
