use std::fs;
use std::path::Path;
use std::path::PathBuf;

use codex_protocol::ThreadId;
use codex_protocol::protocol::HistoryPosition;
use pretty_assertions::assert_eq;
use tempfile::TempDir;
use uuid::Uuid;

use super::RolloutReferenceIndex;

#[tokio::test]
async fn scans_active_archived_and_compressed_history_bases() -> anyhow::Result<()> {
    let home = TempDir::new()?;
    let source_id = thread_id(Uuid::from_u128(1))?;
    let active_base = history_position(source_id);
    let archived_base = history_position(source_id);
    let compressed_base = history_position(source_id);
    let active_child_id = thread_id(Uuid::from_u128(2))?;
    let archived_child_id = thread_id(Uuid::from_u128(3))?;
    let compressed_child_id = thread_id(Uuid::from_u128(4))?;

    write_rollout(
        active_rollout_path(home.path(), Uuid::from_u128(2)),
        active_child_id,
        Some(active_base),
    )?;
    write_rollout(
        archived_rollout_path(home.path(), Uuid::from_u128(3)),
        archived_child_id,
        Some(archived_base),
    )?;
    let compressed_path = archived_rollout_path(home.path(), Uuid::from_u128(4));
    write_rollout(
        compressed_path.clone(),
        compressed_child_id,
        Some(compressed_base),
    )?;
    compress_now(compressed_path.as_path())?;

    let index = RolloutReferenceIndex::scan(home.path()).await?;

    assert_eq!(index.reference_count(source_id), 3);
    assert_eq!(index.history_base(active_child_id), Some(&active_base));
    assert_eq!(index.history_base(archived_child_id), Some(&archived_base));
    assert_eq!(
        index.history_base(compressed_child_id),
        Some(&compressed_base)
    );
    Ok(())
}

#[tokio::test]
async fn active_duplicate_wins_without_double_counting() -> anyhow::Result<()> {
    let home = TempDir::new()?;
    let active_source_id = thread_id(Uuid::from_u128(8))?;
    let archived_source_id = thread_id(Uuid::from_u128(9))?;
    let child_uuid = Uuid::from_u128(10);
    let child_id = thread_id(Uuid::from_u128(10))?;
    let active_base = history_position(active_source_id);
    write_rollout(
        active_rollout_path(home.path(), child_uuid),
        child_id,
        Some(active_base),
    )?;
    write_rollout(
        archived_rollout_path(home.path(), child_uuid),
        child_id,
        Some(history_position(archived_source_id)),
    )?;

    let index = RolloutReferenceIndex::scan(home.path()).await?;

    assert_eq!(index.history_base(child_id), Some(&active_base));
    assert_eq!(index.reference_count(active_source_id), 1);
    assert_eq!(index.reference_count(archived_source_id), 0);
    Ok(())
}

#[tokio::test]
async fn indexes_multiple_rollouts_for_the_same_thread() -> anyhow::Result<()> {
    let home = TempDir::new()?;
    let source_rollout_id = thread_id(Uuid::from_u128(21))?;
    let replacement_rollout_id = thread_id(Uuid::from_u128(22))?;
    let thread_id = thread_id(Uuid::from_u128(20))?;
    let history_base = history_position(source_rollout_id);
    write_rollout(
        active_rollout_path(home.path(), Uuid::from_u128(21)),
        thread_id,
        Some(history_base),
    )?;
    write_rollout(
        active_rollout_path(home.path(), Uuid::from_u128(22)),
        thread_id,
        Some(history_base),
    )?;

    let index = RolloutReferenceIndex::scan(home.path()).await?;
    assert_eq!(index.history_base(source_rollout_id), Some(&history_base));
    assert_eq!(
        index.history_base(replacement_rollout_id),
        Some(&history_base)
    );
    assert_eq!(index.reference_count(source_rollout_id), 1);
    Ok(())
}

#[tokio::test]
async fn unarchived_scan_finds_all_active_rollouts_owned_by_a_thread() -> anyhow::Result<()> {
    let home = TempDir::new()?;
    let owner_id = thread_id(Uuid::from_u128(30))?;
    let original_uuid = Uuid::from_u128(31);
    let replacement_uuid = Uuid::from_u128(32);
    let original_path = active_rollout_path(home.path(), original_uuid);
    let replacement_path = active_rollout_path(home.path(), replacement_uuid);
    write_rollout(original_path.clone(), owner_id, /*history_base*/ None)?;
    compress_now(&original_path)?;
    write_rollout(
        replacement_path.clone(),
        owner_id,
        /*history_base*/ None,
    )?;
    write_rollout(
        archived_rollout_path(home.path(), Uuid::from_u128(33)),
        owner_id,
        /*history_base*/ None,
    )?;
    write_rollout(
        active_rollout_path(home.path(), Uuid::from_u128(34)),
        thread_id(Uuid::from_u128(34))?,
        /*history_base*/ None,
    )?;

    let index = RolloutReferenceIndex::scan_unarchived(home.path()).await?;
    let mut owned: Vec<_> = index
        .rollouts_for_thread(owner_id)
        .map(|(id, path)| (id, path.to_path_buf()))
        .collect();
    owned.sort_by_key(|(_, path)| path.clone());
    assert_eq!(
        owned,
        vec![
            (
                thread_id(original_uuid)?,
                original_path.with_extension("jsonl.zst")
            ),
            (thread_id(replacement_uuid)?, replacement_path),
        ]
    );
    Ok(())
}

#[tokio::test]
async fn filtered_unarchived_scan_includes_reverts_and_requested_descendants() -> anyhow::Result<()>
{
    let home = TempDir::new()?;
    let owner_uuid = Uuid::from_u128(40);
    let owner_id = thread_id(owner_uuid)?;
    let replacement_id = thread_id(Uuid::from_u128(41))?;
    let descendant_uuid = Uuid::from_u128(42);
    let descendant_id = thread_id(descendant_uuid)?;
    let unrelated_uuid = Uuid::from_u128(43);
    let unrelated_id = thread_id(unrelated_uuid)?;
    let original_path = active_rollout_path(home.path(), owner_uuid);
    let replacement_path = original_path.with_file_name(format!(
        "rollout-2025-01-03T12-00-00-{owner_id}_{replacement_id}.jsonl"
    ));
    let descendant_path = active_rollout_path(home.path(), descendant_uuid);

    write_rollout(original_path.clone(), owner_id, /*history_base*/ None)?;
    compress_now(&original_path)?;
    write_rollout(
        replacement_path.clone(),
        owner_id,
        /*history_base*/ None,
    )?;
    // A leftover compressed sibling must not supersede the plain rollout.
    compress_now(&replacement_path)?;
    write_rollout(
        replacement_path.clone(),
        owner_id,
        /*history_base*/ None,
    )?;
    write_rollout(
        descendant_path.clone(),
        descendant_id,
        Some(history_position(owner_id)),
    )?;
    write_rollout(
        archived_rollout_path(home.path(), owner_uuid),
        owner_id,
        /*history_base*/ None,
    )?;
    let unrelated_path = active_rollout_path(home.path(), unrelated_uuid);
    write_rollout(
        unrelated_path.clone(),
        unrelated_id,
        /*history_base*/ None,
    )?;
    compress_now(&unrelated_path)?;

    let index =
        RolloutReferenceIndex::scan_unarchived_threads(home.path(), &[owner_id, descendant_id])
            .await?;
    let mut owned: Vec<_> = index
        .rollouts_for_thread(owner_id)
        .map(|(id, path)| (id, path.to_path_buf()))
        .collect();
    owned.sort_by_key(|(id, _)| id.to_string());
    assert_eq!(
        owned,
        vec![
            (owner_id, original_path.with_extension("jsonl.zst")),
            (replacement_id, replacement_path),
        ]
    );
    assert_eq!(
        index.rollouts_for_thread(descendant_id).collect::<Vec<_>>(),
        vec![(descendant_id, descendant_path.as_path())]
    );
    assert_eq!(
        index.rollouts_for_thread(unrelated_id).collect::<Vec<_>>(),
        Vec::new()
    );
    Ok(())
}

#[tokio::test]
async fn filtered_unarchived_scan_validates_candidate_ownership() -> anyhow::Result<()> {
    let home = TempDir::new()?;
    let requested_uuid = Uuid::from_u128(50);
    let requested_id = thread_id(requested_uuid)?;
    let actual_owner = thread_id(Uuid::from_u128(51))?;
    write_rollout(
        active_rollout_path(home.path(), requested_uuid),
        actual_owner,
        /*history_base*/ None,
    )?;

    let index =
        RolloutReferenceIndex::scan_unarchived_threads(home.path(), &[requested_id]).await?;
    assert_eq!(
        index.rollouts_for_thread(requested_id).collect::<Vec<_>>(),
        Vec::new()
    );
    Ok(())
}

/// Restoring a thread must include its compressed/reverted files without scanning other history.
#[tokio::test]
async fn filtered_archived_scan_includes_only_owned_archived_candidates() -> anyhow::Result<()> {
    let home = TempDir::new()?;
    let owner_uuid = Uuid::from_u128(60);
    let owner_id = thread_id(owner_uuid)?;
    let replacement_id = thread_id(Uuid::from_u128(61))?;
    let other_uuid = Uuid::from_u128(62);
    let other_id = thread_id(other_uuid)?;
    let mismatched_id = thread_id(Uuid::from_u128(63))?;
    let original_path = archived_rollout_path(home.path(), owner_uuid);
    let replacement_path = original_path.with_file_name(format!(
        "rollout-2025-01-03T12-00-00-{owner_id}_{replacement_id}.jsonl"
    ));
    write_rollout(original_path.clone(), owner_id, /*history_base*/ None)?;
    compress_now(&original_path)?;
    write_rollout(
        replacement_path.clone(),
        owner_id,
        Some(history_position(owner_id)),
    )?;

    // A matching filename is only a candidate: metadata must confirm ownership.
    let mismatched_path = original_path.with_file_name(format!(
        "rollout-2025-01-03T12-00-00-{owner_id}_{mismatched_id}.jsonl"
    ));
    write_rollout(mismatched_path, other_id, /*history_base*/ None)?;
    // Unrelated filenames are excluded before reading their metadata.
    write_rollout(
        archived_rollout_path(home.path(), other_uuid),
        owner_id,
        /*history_base*/ None,
    )?;
    write_rollout(
        active_rollout_path(home.path(), owner_uuid),
        owner_id,
        /*history_base*/ None,
    )?;

    let index = RolloutReferenceIndex::scan_archived_threads(home.path(), &[owner_id]).await?;
    let mut owned: Vec<_> = index
        .rollouts_for_thread(owner_id)
        .map(|(id, path)| (id, path.to_path_buf()))
        .collect();
    owned.sort_by_key(|(id, _)| id.to_string());
    assert_eq!(
        owned,
        vec![
            (owner_id, original_path.with_extension("jsonl.zst")),
            (replacement_id, replacement_path),
        ]
    );
    Ok(())
}

#[tokio::test]
async fn self_history_base_does_not_count_as_reference() -> anyhow::Result<()> {
    let home = TempDir::new()?;
    let thread_id = thread_id(Uuid::from_u128(11))?;
    let history_base = history_position(thread_id);
    write_rollout(
        active_rollout_path(home.path(), Uuid::from_u128(11)),
        thread_id,
        Some(history_base),
    )?;

    let index = RolloutReferenceIndex::scan(home.path()).await?;

    assert_eq!(index.history_base(thread_id), Some(&history_base));
    assert_eq!(index.reference_count(thread_id), 0);
    Ok(())
}

fn active_rollout_path(home: &Path, uuid: Uuid) -> PathBuf {
    home.join("sessions/2025/01/03")
        .join(format!("rollout-2025-01-03T12-00-00-{uuid}.jsonl"))
}

fn archived_rollout_path(home: &Path, uuid: Uuid) -> PathBuf {
    home.join("archived_sessions")
        .join(format!("rollout-2025-01-03T12-00-00-{uuid}.jsonl"))
}

fn write_rollout(
    path: PathBuf,
    thread_id: ThreadId,
    history_base: Option<HistoryPosition>,
) -> anyhow::Result<()> {
    fs::create_dir_all(path.parent().expect("rollout parent"))?;
    let session_meta = serde_json::json!({
        "timestamp": "2025-01-03T12:00:00Z",
        "type": "session_meta",
        "payload": {
            "id": thread_id,
            "timestamp": "2025-01-03T12:00:00Z",
            "cwd": path.parent().expect("rollout parent"),
            "originator": "test",
            "cli_version": "test",
            "source": "cli",
            "model_provider": "test-provider",
            "history_mode": "paginated",
            "history_base": history_base,
        },
    });
    fs::write(path, format!("{session_meta}\n"))?;
    Ok(())
}

fn history_position(rollout_id: ThreadId) -> HistoryPosition {
    HistoryPosition {
        thread_id: rollout_id,
        end_ordinal_exclusive: 2,
        end_byte_offset: 100,
    }
}

fn thread_id(uuid: Uuid) -> anyhow::Result<ThreadId> {
    Ok(ThreadId::from_string(&uuid.to_string())?)
}

fn compress_now(path: &Path) -> anyhow::Result<()> {
    let compressed_path = path.with_extension("jsonl.zst");
    let input = fs::File::open(path)?;
    let output = fs::File::create(compressed_path)?;
    let mut encoder = zstd::stream::write::Encoder::new(output, 3)?;
    std::io::copy(&mut std::io::BufReader::new(input), &mut encoder)?;
    encoder.finish()?;
    fs::remove_file(path)?;
    Ok(())
}
