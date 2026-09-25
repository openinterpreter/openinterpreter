//! Serializes enrollment storage across login sessions.
//! An admitted SQLite write keeps its permit until completion even if its caller is cancelled.
//! Process shutdown drains admitted writes after stopping the session workers.

use super::RemoteControlSession;
use super::auth::RemoteControlAuth;
use super::desired_state::RemoteControlDesiredState;
use super::enroll::RemoteControlEnrollment;
use super::enroll::update_persisted_remote_control_enrollment;
use super::protocol::RemoteControlTarget;
use codex_state::StateRuntime;
use std::future::Future;
use std::io;
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio::sync::SemaphorePermit;
use tokio::sync::watch;
use tokio_util::task::TaskTracker;

#[cfg(test)]
#[path = "persistence_tests.rs"]
mod tests;

#[derive(Clone)]
pub(super) struct RemoteControlPersistence {
    semaphore: Arc<Semaphore>,
    pub(super) tasks: TaskTracker,
}

impl Default for RemoteControlPersistence {
    fn default() -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(1)),
            tasks: TaskTracker::new(),
        }
    }
}

impl RemoteControlPersistence {
    pub(super) async fn lock(&self) -> SemaphorePermit<'_> {
        self.semaphore
            .acquire()
            .await
            .unwrap_or_else(|_| unreachable!())
    }
}

pub(super) async fn read_lock<'a>(
    auth: &RemoteControlAuth,
    lock: &'a RemoteControlPersistence,
) -> io::Result<SemaphorePermit<'a>> {
    let permit = lock.lock().await;
    auth.ensure_current()?;
    Ok(permit)
}

async fn commit<T: Send + 'static>(
    auth: &RemoteControlAuth,
    lock: &RemoteControlPersistence,
    operation: impl Future<Output = io::Result<T>> + Send + 'static,
) -> io::Result<T> {
    // Register before checking admission. Shutdown either observes this token or rejects us.
    let task = lock.tasks.token();
    if lock.tasks.is_closed() {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "remote control is shutting down",
        ));
    }
    let auth = auth.clone();
    let semaphore = lock.semaphore.clone();
    tokio::spawn(async move {
        let _task = task;
        let _permit = semaphore.acquire_owned().await.map_err(io::Error::other)?;
        auth.ensure_current()?;
        operation.await
    })
    .await
    .map_err(io::Error::other)?
}

pub(super) async fn save_enrollment(
    auth: &RemoteControlAuth,
    lock: &RemoteControlPersistence,
    state_db: &StateRuntime,
    enrollment: &RemoteControlEnrollment,
    client_name: Option<&str>,
    desired: &watch::Sender<RemoteControlDesiredState>,
) -> io::Result<()> {
    let state_db = state_db.clone();
    let enrollment = enrollment.clone();
    let client_name = client_name.map(str::to_owned);
    let desired = desired.clone();
    commit(auth, lock, async move {
        let preference = match *desired.borrow() {
            RemoteControlDesiredState::Enabled {
                persistence_preference,
            } => persistence_preference,
            RemoteControlDesiredState::Disabled | RemoteControlDesiredState::Unknown => {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "remote control disabled during enrollment",
                ));
            }
        };
        update_persisted_remote_control_enrollment(
            Some(&state_db),
            &enrollment.remote_control_target,
            &enrollment.account_id,
            client_name.as_deref(),
            Some(&enrollment),
            preference,
        )
        .await
    })
    .await
}

impl RemoteControlSession {
    pub(super) async fn set_preference(
        &self,
        state_db: &StateRuntime,
        target: &RemoteControlTarget,
        account_id: &str,
        client_name: Option<&str>,
        enabled: bool,
        fallback_enrollment: Option<&RemoteControlEnrollment>,
    ) -> io::Result<()> {
        let state_db = state_db.clone();
        let target = target.clone();
        let account_id = account_id.to_owned();
        let client_name = client_name.map(str::to_owned);
        let enrollment = fallback_enrollment.cloned();
        let desired = self.desired_state_tx.as_ref().clone();
        commit(&self.auth_manager, &self.persistence, async move {
            let updated = state_db
                .set_remote_control_enabled(
                    &target.websocket_url,
                    &account_id,
                    client_name.as_deref(),
                    enabled,
                )
                .await
                .map_err(io::Error::other)?;
            if updated == 0
                && let Some(enrollment) = enrollment
            {
                update_persisted_remote_control_enrollment(
                    Some(&state_db),
                    &target,
                    &account_id,
                    client_name.as_deref(),
                    Some(&enrollment),
                    Some(enabled),
                )
                .await?;
            }
            if !enabled {
                desired.send_replace(RemoteControlDesiredState::Disabled);
            }
            Ok(())
        })
        .await
    }
}
