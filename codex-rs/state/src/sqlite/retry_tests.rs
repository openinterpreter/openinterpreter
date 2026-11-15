//! Tests safe retries against real SQLite writer contention.

use super::retry_on_busy;
use crate::SqliteConfig;
use crate::SqliteQueueStore;
use crate::migrations::runtime_logs_migrator;
use crate::migrations::runtime_queue_migrator;
use crate::runtime::test_support::unique_temp_dir;
use anyhow::Context;
use codex_protocol::ThreadId;
use codex_utils_absolute_path::test_support::PathExt;
use pretty_assertions::assert_eq;
use sqlx::sqlite::SqlitePoolOptions;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::sync::mpsc;

#[tokio::test]
#[expect(
    clippy::await_holding_invalid_type,
    reason = "Only this sequential retry closure owns these connections. The guards keep the real competing SQLite lock alive across attempts; no other task takes either guard."
)]
async fn retries_after_a_competing_writer_releases_its_lock() -> anyhow::Result<()> {
    let sqlite_home = unique_temp_dir();
    tokio::fs::create_dir_all(&sqlite_home).await?;
    let _cleanup = scopeguard::guard(sqlite_home.clone(), |sqlite_home| {
        let _ = std::fs::remove_dir_all(sqlite_home);
    });
    let sqlite = SqliteConfig::new_for_testing(sqlite_home.as_path().abs());
    let pool = sqlite
        .open_logs_db(&runtime_logs_migrator(), /*telemetry_override*/ None)
        .await?;
    let mut competing_writer = pool.acquire().await?;
    let mut writer = pool.acquire().await?;
    sqlx::query("PRAGMA busy_timeout=0")
        .execute(&mut *writer)
        .await?;
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut *competing_writer)
        .await?;
    let competing_writer = Mutex::new(competing_writer);
    let writer = Mutex::new(writer);

    retry_on_busy(|| async {
        let mut writer = writer.lock().await;
        let result = sqlx::query(
            "INSERT INTO logs (ts, ts_nanos, level, target, feedback_log_body) \
             VALUES (1, 0, 'INFO', 'retry-test', 'saved once')",
        )
        .execute(&mut **writer)
        .await;
        // Release the competing transaction only after SQLite actually rejected
        // the write. This cannot pass by racing the first attempt's busy wait.
        if result.is_err() {
            let mut competing_writer = competing_writer.lock().await;
            sqlx::query("ROLLBACK")
                .execute(&mut **competing_writer)
                .await?;
        }
        result.map_err(anyhow::Error::from)
    })
    .await?;

    let rows = sqlx::query_scalar::<_, String>("SELECT feedback_log_body FROM logs ORDER BY id")
        .fetch_all(&pool)
        .await?;
    assert_eq!(rows, vec!["saved once".to_string()]);
    drop(writer);
    drop(competing_writer);
    pool.close().await;
    Ok(())
}

#[tokio::test]
async fn queue_enqueue_survives_received_busy_without_duplicating_the_message() -> anyhow::Result<()>
{
    let sqlite_home = unique_temp_dir();
    tokio::fs::create_dir_all(&sqlite_home).await?;
    let _cleanup = scopeguard::guard(sqlite_home.clone(), |sqlite_home| {
        let _ = std::fs::remove_dir_all(sqlite_home);
    });
    let sqlite = SqliteConfig::new_for_testing(sqlite_home.as_path().abs());
    let pool = sqlite
        .open_queue_db(&runtime_queue_migrator(), /*telemetry_override*/ None)
        .await?;
    let (attempt_finished, mut finished) = mpsc::unbounded_channel();
    // No connection is opened or released until enqueue's first attempt.
    // SQLite must have returned BUSY before after_release can run: the other
    // writer is still holding the lock, and this connection never waits for it.
    let writer_pool = SqlitePoolOptions::new()
        .max_connections(/*max*/ 1)
        .after_release(move |_, _| {
            let _ = attempt_finished.send(());
            Box::pin(async { Ok(true) })
        })
        .connect_lazy_with(
            (*pool.connect_options())
                .clone()
                .busy_timeout(Duration::ZERO),
        );
    let queue = SqliteQueueStore::new(Arc::new(writer_pool));
    let observer = SqliteQueueStore::new(Arc::new(pool.clone()));
    let thread_id = ThreadId::new();
    let mut competing_writer = pool.acquire().await?;
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut *competing_writer)
        .await?;
    let (item, ()) = tokio::try_join!(
        queue.enqueue(thread_id, r#"{"text":"save this bound value"}"#),
        async {
            finished
                .recv()
                .await
                .context("no completed writer attempt")?;
            sqlx::query("ROLLBACK")
                .execute(&mut *competing_writer)
                .await?;
            Ok::<(), anyhow::Error>(())
        }
    )?;
    assert_eq!(
        (thread_id, r#"{"text":"save this bound value"}"#),
        (item.thread_id, item.payload.as_str())
    );
    assert_eq!(
        vec![item],
        observer
            .list_page(thread_id, /*offset*/ 0, /*limit*/ 10)
            .await?
    );
    drop(competing_writer);
    queue.close().await;
    pool.close().await;
    Ok(())
}

#[tokio::test]
async fn queue_update_survives_received_busy_and_commits_once() -> anyhow::Result<()> {
    let sqlite_home = unique_temp_dir();
    tokio::fs::create_dir_all(&sqlite_home).await?;
    let _cleanup = scopeguard::guard(sqlite_home.clone(), |sqlite_home| {
        let _ = std::fs::remove_dir_all(sqlite_home);
    });
    let sqlite = SqliteConfig::new_for_testing(sqlite_home.as_path().abs());
    let pool = sqlite
        .open_queue_db(&runtime_queue_migrator(), /*telemetry_override*/ None)
        .await?;
    let observer = SqliteQueueStore::new(Arc::new(pool.clone()));
    let thread_id = ThreadId::new();
    let mut expected = observer
        .enqueue(thread_id, r#"{"text":"original"}"#)
        .await?;
    let revision_before = observer.changes_since(/*revision*/ 0, &[thread_id]).await?[0].1;
    expected.payload = r#"{"text":"edited bound value"}"#.to_string();
    let (attempt_finished, mut finished) = mpsc::unbounded_channel();
    let writer_pool = SqlitePoolOptions::new()
        .max_connections(/*max*/ 1)
        .after_release(move |_, _| {
            let _ = attempt_finished.send(());
            Box::pin(async { Ok(true) })
        })
        .connect_lazy_with(
            (*pool.connect_options())
                .clone()
                .busy_timeout(Duration::ZERO),
        );
    let queue = SqliteQueueStore::new(Arc::new(writer_pool));
    let mut competing_writer = pool.acquire().await?;
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut *competing_writer)
        .await?;
    let (updated, ()) = tokio::try_join!(
        queue.update(thread_id, &expected.id, &expected.payload),
        async {
            // Release only after update actually failed admission, not while
            // SQLite's busy handler could still turn the first attempt into success.
            finished
                .recv()
                .await
                .context("no completed writer attempt")?;
            sqlx::query("ROLLBACK")
                .execute(&mut *competing_writer)
                .await?;
            Ok::<(), anyhow::Error>(())
        }
    )?;
    assert_eq!(Some(expected.clone()), updated);
    assert_eq!(
        (vec![expected], vec![(thread_id, revision_before + 1)]),
        (
            observer
                .list_page(thread_id, /*offset*/ 0, /*limit*/ 10)
                .await?,
            observer
                .changes_since(revision_before, &[thread_id])
                .await?
        )
    );
    drop(competing_writer);
    queue.close().await;
    pool.close().await;
    Ok(())
}

#[tokio::test]
async fn queue_delete_survives_received_busy_and_preserves_other_messages() -> anyhow::Result<()> {
    let sqlite_home = unique_temp_dir();
    tokio::fs::create_dir_all(&sqlite_home).await?;
    let _cleanup = scopeguard::guard(sqlite_home.clone(), |sqlite_home| {
        let _ = std::fs::remove_dir_all(sqlite_home);
    });
    let sqlite = SqliteConfig::new_for_testing(sqlite_home.as_path().abs());
    let pool = sqlite
        .open_queue_db(&runtime_queue_migrator(), /*telemetry_override*/ None)
        .await?;
    let observer = SqliteQueueStore::new(Arc::new(pool.clone()));
    let thread_id = ThreadId::new();
    let target = observer.enqueue(thread_id, r#"{"text":"remove"}"#).await?;
    let survivor = observer.enqueue(thread_id, r#"{"text":"keep"}"#).await?;
    let revision_before = observer.changes_since(/*revision*/ 0, &[thread_id]).await?[0].1;
    let (attempt_finished, mut finished) = mpsc::unbounded_channel();
    let writer_pool = SqlitePoolOptions::new()
        .max_connections(/*max*/ 1)
        .after_release(move |_, _| {
            let _ = attempt_finished.send(());
            Box::pin(async { Ok(true) })
        })
        .connect_lazy_with(
            (*pool.connect_options())
                .clone()
                .busy_timeout(Duration::ZERO),
        );
    let queue = SqliteQueueStore::new(Arc::new(writer_pool));
    let mut competing_writer = pool.acquire().await?;
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut *competing_writer)
        .await?;
    let (deleted, ()) = tokio::try_join!(queue.delete(thread_id, &target.id), async {
        // The first autocommit DELETE has returned BUSY and released its
        // connection. Its bound thread and item IDs must survive the retry.
        finished
            .recv()
            .await
            .context("no completed writer attempt")?;
        sqlx::query("ROLLBACK")
            .execute(&mut *competing_writer)
            .await?;
        Ok::<(), anyhow::Error>(())
    })?;
    assert!(deleted);
    assert_eq!(
        (vec![survivor], vec![(thread_id, revision_before + 1)]),
        (
            observer
                .list_page(thread_id, /*offset*/ 0, /*limit*/ 10)
                .await?,
            observer
                .changes_since(revision_before, &[thread_id])
                .await?
        )
    );
    drop(competing_writer);
    queue.close().await;
    pool.close().await;
    Ok(())
}
