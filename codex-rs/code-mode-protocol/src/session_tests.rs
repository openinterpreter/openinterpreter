use pretty_assertions::assert_eq;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use super::CellId;
use super::CodeModeSession;
use super::CodeModeSessionDelegate;
use super::CodeModeSessionResultFuture;
use super::NoopCodeModeSessionDelegate;
use super::StartedCell;
use crate::ExecuteRequest;
use crate::TerminateRequest;
use crate::WaitOutcome;
use crate::WaitRequest;

struct OrdinarySession(AtomicUsize);

impl CodeModeSession for OrdinarySession {
    fn execute<'a>(
        &'a self,
        _request: ExecuteRequest,
        _delegate: Arc<dyn CodeModeSessionDelegate>,
    ) -> CodeModeSessionResultFuture<'a, StartedCell> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Box::pin(async {
            Ok(StartedCell::from_future(
                CellId::new("cell".into()),
                async { Err("fixture response unavailable".into()) },
            ))
        })
    }

    fn wait<'a>(
        &'a self,
        _request: WaitRequest,
        _preempt: Option<CancellationToken>,
    ) -> CodeModeSessionResultFuture<'a, WaitOutcome> {
        Box::pin(async { Err("unused fixture".into()) })
    }

    fn terminate<'a>(
        &'a self,
        _request: TerminateRequest,
    ) -> CodeModeSessionResultFuture<'a, WaitOutcome> {
        Box::pin(async { Err("unused fixture".into()) })
    }

    fn shutdown<'a>(&'a self) -> CodeModeSessionResultFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
}

#[tokio::test]
async fn preemption_is_never_silently_ignored_by_ordinary_session() {
    let session = OrdinarySession(AtomicUsize::new(0));
    let request = || ExecuteRequest {
        tool_call_id: "test".into(),
        enabled_tools: Vec::new(),
        source: "1 + 1".into(),
        yield_time_ms: None,
        max_output_tokens: None,
    };
    let delegate: Arc<dyn CodeModeSessionDelegate> = Arc::new(NoopCodeModeSessionDelegate);
    assert_eq!(
        session
            .execute_with_preempt(
                request(),
                Arc::clone(&delegate),
                Some(CancellationToken::new())
            )
            .await
            .err()
            .as_deref(),
        Some("code-mode session does not support foreground preemption")
    );
    assert_eq!(session.0.load(Ordering::Relaxed), 0);
    assert!(
        session
            .execute_with_preempt(request(), delegate, None)
            .await
            .is_ok()
    );
    assert_eq!(session.0.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn started_cell_preserves_remote_initial_response_errors() {
    let (response_tx, response_rx) = oneshot::channel();
    response_tx
        .send(Err("remote runtime failed".to_string()))
        .expect("initial response receiver should be open");
    let started = StartedCell::from_result_receiver(CellId::new("1".to_string()), response_rx);

    assert_eq!(
        started.initial_response().await,
        Err("remote runtime failed".to_string())
    );
}
