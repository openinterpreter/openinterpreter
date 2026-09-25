//! Cancels device ceremonies when cached authentication changes.

use codex_login::AuthManager;

use super::*;

#[derive(Clone, PartialEq, Eq)]
pub(super) struct Identity {
    user_id: String,
    account_id: String,
}

impl OutgoingMessageSender {
    /// Trusted host activation enables a UI integration after its signing path is available.
    pub(crate) async fn enable_user_verification_connection(&self, connection_id: ConnectionId) {
        self.verification_connections
            .lock()
            .await
            .insert(connection_id);
    }

    /// Stop routing ceremonies immediately, without waiting for unrelated RPCs to drain.
    pub(crate) async fn disconnect_user_verification_connection(
        &self,
        connection_id: ConnectionId,
    ) {
        self.verification_connections
            .lock()
            .await
            .remove(&connection_id);
        self.request_id_to_callback
            .lock()
            .await
            .retain(|_, entry| entry.verification_owner != Some(connection_id));
    }

    pub(crate) fn watch_user_verification_auth(self: &Arc<Self>, auth_manager: Arc<AuthManager>) {
        let mut changes = auth_manager.auth_change_receiver();
        if self.verification_auth.set(auth_manager).is_err() {
            return;
        }
        let outgoing = Arc::downgrade(self);
        tokio::spawn(async move {
            while changes.changed().await.is_ok() {
                let Some(outgoing) = outgoing.upgrade() else {
                    break;
                };
                // Compare revisions, since watch can coalesce an account switch and switch back.
                // Requests created after the change already belong to the new auth revision.
                let mut callbacks = outgoing.request_id_to_callback.lock().await;
                let revision = outgoing.verification_auth_revision();
                callbacks.retain(|_, entry| {
                    entry.verification_owner.is_none()
                        || entry.verification_auth_revision == revision
                });
            }
        });
    }

    pub(super) fn verification_identity(&self) -> Option<Identity> {
        let auth = self.verification_auth.get()?.auth_cached()?;
        Some(Identity {
            user_id: auth.get_chatgpt_user_id()?,
            account_id: auth.get_account_id()?,
        })
    }

    pub(super) fn verification_auth_revision(&self) -> Option<u64> {
        let changes = self.verification_auth.get()?.auth_change_receiver();
        Some(*changes.borrow())
    }
}
