//! Recovers lost tmux resize notifications without querying geometry on the UI thread.
//!
//! One worker belongs to the event broker, including across nested screens. Queries never
//! hold the shared lock. Pauses and newer geometry invalidate both in-flight and queued
//! samples. Dropping the monitor wakes the worker without joining a possibly blocked OS call;
//! that single in-flight call may finish later, but cannot publish or start another query.

use std::io;
use std::sync::Arc;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use codex_terminal_detection::Multiplexer;
use codex_terminal_detection::TerminalInfo;
use ratatui::layout::Size;
use tokio::sync::broadcast;

const CHECK_INTERVAL: Duration = Duration::from_millis(500);

struct State {
    size: Size,
    generation: u64,
    pending: Option<Size>,
    active: bool,
    stopped: bool,
}

pub(super) struct SizeMonitor {
    state: Arc<Mutex<State>>,
    worker: thread::JoinHandle<()>,
}

impl SizeMonitor {
    pub(super) fn start(
        terminal_info: &TerminalInfo,
        size: Size,
        draw_tx: broadcast::Sender<()>,
    ) -> Option<Self> {
        if !cfg!(unix) || !matches!(terminal_info.multiplexer, Some(Multiplexer::Tmux { .. })) {
            return None;
        }
        Self::spawn(size, draw_tx, || {
            // Unlike terminal::size(), window_size() has no tput subprocess fallback.
            crossterm::terminal::window_size().map(|size| Size::new(size.columns, size.rows))
        })
        .inspect_err(|error| tracing::warn!(%error, "Failed to start terminal size monitor"))
        .ok()
    }

    fn spawn(
        size: Size,
        draw_tx: broadcast::Sender<()>,
        mut query: impl FnMut() -> io::Result<Size> + Send + 'static,
    ) -> io::Result<Self> {
        let state = Arc::new(Mutex::new(State {
            size,
            generation: 0,
            pending: None,
            active: true,
            stopped: false,
        }));
        let shared = state.clone();
        let worker = thread::Builder::new()
            .name("codex-terminal-size".into())
            .spawn(move || {
                loop {
                    thread::park_timeout(CHECK_INTERVAL);
                    let generation = {
                        let state = shared
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        if state.stopped {
                            break;
                        }
                        if !state.active {
                            continue;
                        }
                        state.generation
                    };
                    let sampled = query();
                    let mut state = shared
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    if state.stopped {
                        break;
                    }
                    if state.generation != generation {
                        continue;
                    }
                    state.pending = sampled
                        .ok()
                        .filter(|size| size.width > 0 && size.height > 0 && *size != state.size);
                    if state.pending.is_some() {
                        drop(state);
                        let _ = draw_tx.send(());
                    }
                }
            })?;
        Ok(Self { state, worker })
    }

    pub(super) fn observe(&self, size: Size) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.size = size;
        state.generation = state.generation.wrapping_add(1);
        state.pending = None;
    }

    pub(super) fn set_active(&self, active: bool) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.active = active;
        state.generation = state.generation.wrapping_add(1);
        state.pending = None;
    }

    pub(super) fn take_resize(&self) -> Option<Size> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let size = state.pending.take()?;
        state.size = size;
        state.generation = state.generation.wrapping_add(1);
        tracing::debug!(?size, "Recovered terminal size from background check");
        Some(size)
    }
}

impl Drop for SizeMonitor {
    fn drop(&mut self) {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .stopped = true;
        self.worker.thread().unpark();
    }
}

#[cfg(test)]
#[path = "size_monitor_tests.rs"]
mod tests;
