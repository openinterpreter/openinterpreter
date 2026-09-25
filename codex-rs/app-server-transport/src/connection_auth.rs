//! Binds a transport connection to the authentication owner that established it.
//! Owner revisions invalidate queued work even before transport closure is delivered.

use codex_login::AuthChangeState;
use codex_login::AuthManager;
use std::io;
use tokio::sync::watch;

#[derive(Clone, Debug)]
pub struct ConnectionAuth {
    changes: watch::Receiver<AuthChangeState>,
    owner_generation: u64,
}

impl ConnectionAuth {
    pub(crate) fn capture(auth_manager: &AuthManager) -> Self {
        let changes = auth_manager.auth_change_state_receiver();
        let owner_generation = changes.borrow().owner_generation;
        Self::new(changes, owner_generation)
    }

    pub(crate) fn ensure_current(&self) -> io::Result<()> {
        if self.is_current() {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "remote control authentication changed",
            ))
        }
    }

    pub(crate) fn new(changes: watch::Receiver<AuthChangeState>, owner_generation: u64) -> Self {
        Self {
            changes,
            owner_generation,
        }
    }

    pub fn is_current(&self) -> bool {
        self.changes.borrow().owner_generation == self.owner_generation
            && self.changes.has_changed().is_ok()
    }

    pub(crate) async fn invalidated(&self) {
        let mut changes = self.changes.clone();
        let _ = changes
            .wait_for(|state| state.owner_generation != self.owner_generation)
            .await;
    }
}
