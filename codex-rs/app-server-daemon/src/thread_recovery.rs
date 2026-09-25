//! Removes stale snapshots before planned daemon replacements.

use std::io::ErrorKind;

use anyhow::Context;
use anyhow::Result;

use crate::Daemon;

pub(crate) fn discard_pending(daemon: &Daemon) -> Result<()> {
    match std::fs::remove_file(daemon.recovery_file()?) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err).context("failed to clear daemon recovery file"),
    }
}
