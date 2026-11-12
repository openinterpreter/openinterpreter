use std::cell::Cell;
use std::fs;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::io::Write;

use pretty_assertions::assert_eq;

use super::*;

#[tokio::test]
async fn dropping_the_caller_stops_compressed_materialization() -> anyhow::Result<()> {
    let home = tempfile::tempdir()?;
    let path = home.path().join("rollout.jsonl.zst");
    let original = vec![b'x'; 1024 * 1024];
    let compressed = zstd::stream::encode_all(original.as_slice(), /*level*/ 3)?;
    fs::write(&path, &compressed)?;

    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (resume_tx, resume_rx) = std::sync::mpsc::channel();
    let (finished_tx, finished_rx) = tokio::sync::oneshot::channel();
    let read_path = path.clone();
    let task = tokio::spawn(async move {
        let (stop, keep_running) = tokio::sync::oneshot::channel::<()>();
        let worker = tokio::task::spawn_blocking(move || {
            let checks = Cell::new(0);
            let started = Cell::new(Some(started_tx));
            let result = open_rollout_seekable_reader(&read_path, || {
                checks.set(checks.get() + 1);
                // Check on open and around the first source read and decoded chunk.
                // Pause after that chunk so a check only before decompression cannot pass.
                if checks.get() >= 5
                    && let Some(started) = started.take()
                {
                    let _ = started.send(());
                    let _ = resume_rx.recv();
                }
                stop.is_closed()
            });
            let _ = finished_tx.send(result.err().map(|error| error.kind()));
        });
        let result = worker.await;
        drop(keep_running);
        result
    });

    started_rx.await?;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    resume_tx.send(())?;
    assert_eq!(finished_rx.await?, Some(io::ErrorKind::Other));
    assert_eq!(fs::read(&path)?, compressed);
    assert!(!path.with_extension("").exists());
    Ok(())
}

#[test]
fn cancellation_stops_compressed_input_and_output() -> io::Result<()> {
    let home = tempfile::tempdir()?;
    let path = home.path().join("rollout.jsonl.zst");
    let original = vec![b'x'; 1024 * 1024];
    let mut skippable = 0x184D2A50_u32.to_le_bytes().to_vec();
    skippable.extend((original.len() as u32).to_le_bytes());
    skippable.extend(&original);
    // Exercise many input reads with no output, and many output reads from little input.
    for bytes in [
        skippable,
        zstd::stream::encode_all(original.as_slice(), /*level*/ 3)?,
    ] {
        fs::write(&path, bytes)?;
        drop(open_rollout_seekable_reader(&path, || false)?);
        let checks = Cell::new(0);
        let cancelled = || checks.replace(checks.get() + 1) >= 10;
        assert!(open_rollout_seekable_reader(&path, cancelled).is_err());
        checks.set(0);
        assert!(rollout_contains_prefix(&path, original.len() as u64, cancelled).is_err());
    }
    Ok(())
}

#[test]
fn read_snapshots_preserve_bytes_and_report_cancellation() -> io::Result<()> {
    let home = tempfile::tempdir()?;
    let path = home.path().join("rollout.jsonl");
    let compressed = path.with_extension("jsonl.zst");
    let original = "first\r\nsecond 🦀\nthird\n".as_bytes();
    fs::write(
        &compressed,
        zstd::stream::encode_all(original, /*level*/ 3)?,
    )?;

    let cancelled = Cell::new(false);
    let is_cancelled = || cancelled.get();
    let mut reader = open_rollout_seekable_reader(&path, is_cancelled)?;
    // The retained snapshot must remain valid after another caller materializes the logical path.
    fs::write(&path, b"replacement\n")?;
    fs::remove_file(&compressed)?;
    reader.seek(SeekFrom::Start(7))?;
    let mut suffix = Vec::new();
    reader.read_to_end(&mut suffix)?;
    assert_eq!(suffix, original[7..]);

    let mut current = Vec::new();
    open_rollout_seekable_reader(&compressed, || false)?.read_to_end(&mut current)?;
    assert_eq!(current, b"replacement\n");

    let mut plain_snapshot = open_rollout_seekable_reader(&path, is_cancelled)?;
    fs::write(
        &compressed,
        zstd::stream::encode_all(current.as_slice(), /*level*/ 3)?,
    )?;
    fs::remove_file(&path)?;
    let mut retained = Vec::new();
    plain_snapshot.read_to_end(&mut retained)?;
    assert_eq!(retained, current);
    reader.rewind()?;
    plain_snapshot.rewind()?;
    cancelled.set(true);
    for mut reader in [reader, plain_snapshot] {
        assert_eq!(
            reader.read_to_end(&mut Vec::new()).unwrap_err().kind(),
            io::ErrorKind::Other
        );
        assert_eq!(reader.rewind().unwrap_err().kind(), io::ErrorKind::Other);
    }
    Ok(())
}

#[test]
fn prefix_bounds_support_sized_unsized_and_concatenated_frames() -> io::Result<()> {
    let home = tempfile::tempdir()?;
    let path = home.path().join("rollout.jsonl.zst");
    let first = "first 🦀\n".as_bytes();
    let second = b"second\n";
    let mut encoder = zstd::stream::write::Encoder::new(Vec::new(), /*level*/ 3)?;
    encoder.set_pledged_src_size(Some(first.len() as u64))?;
    encoder.write_all(first)?;
    let sized = encoder.finish()?;
    let without_size = zstd::stream::encode_all(first, /*level*/ 3)?;
    let mut concatenated = sized.clone();
    concatenated.extend(zstd::stream::encode_all(
        second.as_slice(),
        /*level*/ 3,
    )?);

    for (compressed, expected) in [
        (sized, first.to_vec()),
        (without_size, first.to_vec()),
        (concatenated, [first, second].concat()),
    ] {
        fs::write(&path, compressed)?;
        let contains = |end| rollout_contains_prefix(&path, end, || false);
        assert!(contains(first.len() as u64)?);
        assert!(contains(expected.len() as u64)?);
        assert!(!contains(expected.len() as u64 + 1)?);
        let mut actual = Vec::new();
        open_rollout_seekable_reader(&path, || false)?.read_to_end(&mut actual)?;
        assert_eq!(actual, expected);
        assert!(!path.with_extension("").exists());
    }
    Ok(())
}
