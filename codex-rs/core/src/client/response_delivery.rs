//! Synchronous post-hooks finish before the turn receives response completion.

use crate::client_common::ResponseEvent;
use crate::hook_runtime::sampling::SamplingAttempt;
use codex_api::ApiError;
use codex_hooks::ModelRequestHookEvent;
use codex_protocol::error::CodexErr;
use codex_protocol::error::Result;
use tokio::sync::mpsc;

#[derive(Default)]
pub(super) struct ResponseDelivery {
    attempt: Option<SamplingAttempt>,
}

impl ResponseDelivery {
    pub(super) fn new(attempt: &mut Option<SamplingAttempt>) -> Self {
        Self {
            attempt: attempt.take(),
        }
    }

    /// Return false when the turn has stopped receiving this attempt's output.
    pub(super) async fn send_or_hold(
        &mut self,
        event: Result<ResponseEvent>,
        tx: &mpsc::Sender<Result<ResponseEvent>>,
    ) -> bool {
        let Some(attempt) = self.attempt.as_mut() else {
            return tx.send(event).await.is_ok();
        };
        let report = match &event {
            Ok(ResponseEvent::Completed {
                response_id,
                token_usage,
                ..
            }) => ModelRequestHookEvent::completed(response_id, token_usage.as_ref()),
            Err(error) => ModelRequestHookEvent::failed(error),
            Ok(_) => return tx.send(event).await.is_ok(),
        };
        let owner = attempt.owner_cancellation.clone();
        if let Some(hooks) = attempt.start_post_hooks(report) {
            tokio::select! {
                biased;
                () = tx.closed() => return false,
                () = owner.cancelled() => return false,
                () = hooks => {}
            }
            // The owner can interrupt before the reader is scheduled to close its channel.
            if owner.is_cancelled() {
                return false;
            }
        }
        self.attempt = None;
        let released = Self::release(event, tx).await;
        match released {
            Ok(delivered) => delivered,
            Err(error) => tx.send(Err(codex_api::map_api_error(error))).await.is_ok(),
        }
    }

    async fn release(
        event: Result<ResponseEvent>,
        tx: &mpsc::Sender<Result<ResponseEvent>>,
    ) -> std::result::Result<bool, ApiError> {
        if tx.is_closed() {
            return Ok(false);
        }
        Ok(tx.send(event).await.is_ok())
    }

    pub(super) async fn finish_eof(&mut self, tx: &mpsc::Sender<Result<ResponseEvent>>) {
        let Some(mut attempt) = self.attempt.take() else {
            return;
        };
        let owner = attempt.owner_cancellation.clone();
        if let Some(hooks) = attempt.start_post_hooks(ModelRequestHookEvent::failed(
            &CodexErr::Stream("stream closed before response.completed".into()),
        )) {
            tokio::select! {
                biased;
                () = tx.closed() => {}
                () = owner.cancelled() => {}
                () = hooks => {}
            }
        }
    }
}
