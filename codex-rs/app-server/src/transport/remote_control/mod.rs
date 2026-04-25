mod client_tracker;
mod enroll;
mod protocol;
mod websocket;

use crate::transport::remote_control::websocket::RemoteControlWebsocket;
use crate::transport::remote_control::websocket::load_remote_control_auth;

pub use self::protocol::ClientId;
use self::protocol::ServerEvent;
use self::protocol::StreamId;
use self::protocol::normalize_remote_control_url;
use super::CHANNEL_CAPACITY;
use super::TransportEvent;
use super::next_connection_id;
use codex_login::AuthManager;
use codex_state::StateRuntime;
use std::io;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

pub(super) struct QueuedServerEnvelope {
    pub(super) event: ServerEvent,
    pub(super) client_id: ClientId,
    pub(super) stream_id: StreamId,
    pub(super) write_complete_tx: Option<oneshot::Sender<()>>,
}

#[derive(Clone)]
pub(crate) struct RemoteControlHandle {
    enabled_tx: Arc<watch::Sender<bool>>,
}

impl RemoteControlHandle {
    pub(crate) fn set_enabled(&self, enabled: bool) {
        self.enabled_tx.send_if_modified(|state| {
            let changed = *state != enabled;
            *state = enabled;
            changed
        });
    }
}

pub(crate) async fn start_remote_control(
    remote_control_url: String,
    state_db: Option<Arc<StateRuntime>>,
    auth_manager: Arc<AuthManager>,
    transport_event_tx: mpsc::Sender<TransportEvent>,
    shutdown_token: CancellationToken,
    app_server_client_name_rx: Option<oneshot::Receiver<String>>,
    initial_enabled: bool,
) -> io::Result<(JoinHandle<()>, RemoteControlHandle)> {
    let remote_control_target = if initial_enabled {
        Some(normalize_remote_control_url(&remote_control_url)?)
    } else {
        None
    };

    let (enabled_tx, enabled_rx) = watch::channel(initial_enabled);
    let join_handle = tokio::spawn(async move {
        RemoteControlWebsocket::new(
            remote_control_url,
            remote_control_target,
            state_db,
            auth_manager,
            transport_event_tx,
            shutdown_token,
            enabled_rx,
        )
        .run(app_server_client_name_rx)
        .await;
    });

    Ok((
        join_handle,
        RemoteControlHandle {
            enabled_tx: Arc::new(enabled_tx),
        },
    ))
}

#[cfg(test)]
pub(crate) async fn persist_remote_control_enrollment_for_tests(
    state_db: Option<&StateRuntime>,
    remote_control_url: &str,
    account_id: &str,
    app_server_client_name: Option<&str>,
    server_id: &str,
    environment_id: &str,
    server_name: &str,
) -> io::Result<()> {
    let remote_control_target = normalize_remote_control_url(remote_control_url)?;
    let enrollment = enroll::RemoteControlEnrollment {
        account_id: account_id.to_string(),
        environment_id: environment_id.to_string(),
        server_id: server_id.to_string(),
        server_name: server_name.to_string(),
    };
    enroll::update_persisted_remote_control_enrollment(
        state_db,
        &remote_control_target,
        account_id,
        app_server_client_name,
        Some(&enrollment),
    )
    .await
}

pub(crate) async fn validate_remote_control_auth(
    auth_manager: &Arc<AuthManager>,
) -> io::Result<()> {
    match load_remote_control_auth(auth_manager).await {
        Ok(_) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::WouldBlock => Ok(()),
        Err(err) => Err(err),
    }
}
#[cfg(test)]
mod tests;
