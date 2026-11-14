//! Reads rollout bytes and positions independently of their plain or compressed representation.
//!
//! Offsets always address the original JSONL bytes. Readers retain an open file, or an anonymous
//! decoded snapshot, so a concurrent compression cannot invalidate an in-progress scan.
//! Cancellation is checked during decompression and subsequent bounded reads and seeks.

use std::fs::File;
use std::io;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::path::Path;
use std::time::Duration;

use crate::plain_rollout_path;

enum RolloutReader {
    Plain(File),
    Compressed(File),
}

impl RolloutReader {
    fn open(path: &Path) -> io::Result<Self> {
        let plain_path = plain_rollout_path(path);
        let compressed_path = plain_path.with_extension("jsonl.zst");
        for attempt in 0..4 {
            match File::open(&plain_path) {
                Ok(file) => return Ok(Self::Plain(file)),
                Err(err) if err.kind() == io::ErrorKind::NotFound => {}
                Err(err) => return Err(err),
            }
            match File::open(&compressed_path) {
                Ok(file) => return Ok(Self::Compressed(file)),
                Err(err) if err.kind() == io::ErrorKind::NotFound && attempt < 3 => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(err) => return Err(err),
            }
        }
        unreachable!("the final open attempt returns its error")
    }
}

/// Reads at most `max_bytes` decoded rollout bytes without materializing the durable file.
///
/// Returns `None` for nonregular files. This retains an open representation while reading, so
/// compression or materialization cannot invalidate the read after resolution.
pub fn read_rollout_prefix(path: &Path, max_bytes: usize) -> io::Result<Option<Vec<u8>>> {
    if std::fs::metadata(path).is_ok_and(|metadata| !metadata.is_file()) {
        return Ok(None);
    }
    let source = RolloutReader::open(path)?;
    let file = match &source {
        RolloutReader::Plain(file) | RolloutReader::Compressed(file) => file,
    };
    if !file.metadata()?.is_file() {
        return Ok(None);
    }
    let reader: Box<dyn Read> = match source {
        RolloutReader::Plain(file) => Box::new(file),
        RolloutReader::Compressed(file) => Box::new(zstd::stream::read::Decoder::new(file)?),
    };
    let mut bytes = Vec::new();
    reader.take(max_bytes as u64).read_to_end(&mut bytes)?;
    Ok(Some(bytes))
}

/// Opens the original JSONL bytes for blocking offset reads without changing the rollout on disk.
///
/// Compressed files are decoded into an anonymous temporary file, keeping memory use bounded and
/// preserving read-only access to the Codex home. Plain files retain their existing seek fast path.
/// `is_cancelled` must observe the lifetime of the calling task. Cancellation returns an error,
/// never EOF, so callers cannot mistake a partial read for a complete history.
pub fn open_rollout_seekable_reader(
    path: &Path,
    is_cancelled: impl Fn() -> bool,
) -> io::Result<impl Read + Seek> {
    if is_cancelled() {
        return Err(io::Error::other("rollout read cancelled"));
    }
    let file = match RolloutReader::open(path)? {
        RolloutReader::Plain(file) => file,
        RolloutReader::Compressed(file) => {
            let mut decoded = tempfile::tempfile()?;
            // Empty and skippable frames can consume input without producing decoded bytes.
            let file = CancellableReader {
                inner: file,
                is_cancelled: &is_cancelled,
            };
            let mut reader = CancellableReader {
                inner: zstd::stream::read::Decoder::new(file)?,
                is_cancelled: &is_cancelled,
            };
            io::copy(&mut reader, &mut decoded)?;
            decoded.rewind()?;
            decoded
        }
    };
    Ok(CancellableReader {
        inner: file,
        is_cancelled,
    })
}

struct CancellableReader<R, F> {
    inner: R,
    is_cancelled: F,
}

impl<R, F: Fn() -> bool> CancellableReader<R, F> {
    fn check_cancelled(&self) -> io::Result<()> {
        if (self.is_cancelled)() {
            // Interrupted is retried by io::copy and read_exact, defeating cancellation.
            return Err(io::Error::other("rollout read cancelled"));
        }
        Ok(())
    }
}

impl<R: Read, F: Fn() -> bool> Read for CancellableReader<R, F> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.check_cancelled()?;
        let limit = buffer.len().min(64 * 1024);
        let read = self.inner.read(&mut buffer[..limit])?;
        self.check_cancelled()?;
        Ok(read)
    }
}

impl<R: Seek, F: Fn() -> bool> Seek for CancellableReader<R, F> {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.check_cancelled()?;
        self.inner.seek(position)
    }
}

/// Checks a frozen prefix's byte bound using a blocking read of the logical JSONL representation.
///
/// A known first-frame size is a lower bound even for concatenated zstd frames. New compressed
/// rollouts include that size, so ordinary lineage validation only reads the header. Older frames
/// without a size, and prefixes extending beyond the first frame, are decoded up to the bound.
pub fn rollout_contains_prefix(
    path: &Path,
    end_byte_offset: u64,
    is_cancelled: impl Fn() -> bool,
) -> io::Result<bool> {
    if is_cancelled() {
        return Err(io::Error::other("rollout read cancelled"));
    }
    match RolloutReader::open(path)? {
        RolloutReader::Plain(file) => Ok(end_byte_offset <= file.metadata()?.len()),
        RolloutReader::Compressed(file) => {
            let mut file = CancellableReader {
                inner: file,
                is_cancelled: &is_cancelled,
            };
            // A zstd frame header occupies at most 18 bytes.
            let mut header = [0; 18];
            let read = file.read(&mut header)?;
            if zstd::zstd_safe::get_frame_content_size(&header[..read])
                .ok()
                .flatten()
                .is_some_and(|size| end_byte_offset <= size)
            {
                return Ok(true);
            }
            file.rewind()?;
            let mut prefix = CancellableReader {
                inner: zstd::stream::read::Decoder::new(file)?,
                is_cancelled: &is_cancelled,
            }
            .take(end_byte_offset);
            Ok(io::copy(&mut prefix, &mut io::sink())? == end_byte_offset)
        }
    }
}

#[cfg(test)]
#[path = "seekable_reader_tests.rs"]
mod tests;
