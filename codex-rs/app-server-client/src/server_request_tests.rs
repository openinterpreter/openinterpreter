//! Worker-channel ownership checks, including stale handles and identical request IDs.
use super::*;
use crate::ClientCommand;
use crate::InProcessAppServerRequestHandle;
use pretty_assertions::assert_eq;
use tokio::sync::mpsc;

#[tokio::test]
async fn in_process_replies_keep_their_worker_and_request_id() {
    let (first_tx, mut first_rx) = mpsc::channel(/*buffer*/ 2);
    let (second_tx, mut second_rx) = mpsc::channel(/*buffer*/ 2);
    let first = AppServerRequestHandle::InProcess(InProcessAppServerRequestHandle {
        command_tx: first_tx,
    });
    let second = AppServerRequestHandle::InProcess(InProcessAppServerRequestHandle {
        command_tx: second_tx,
    });
    let id = RequestId::String("private-request".into());
    let reply = first.server_request_reply_handle(id.clone());
    let stale = first.server_request_reply_handle(RequestId::String("stale-request".into()));
    let other = second.server_request_reply_handle(id.clone());
    drop(first);
    assert_eq!(format!("{reply:?}"), "AppServerReplyHandle([redacted])");
    assert_eq!(reply.request_id, id);
    let (result, ()) = tokio::join!(reply.resolve(serde_json::json!({"first": true})), async {
        let ClientCommand::ResolveServerRequest {
            request_id,
            result,
            response_tx,
        } = first_rx.recv().await.unwrap()
        else {
            panic!("wrong command")
        };
        assert_eq!(
            (request_id, result),
            (id.clone(), serde_json::json!({"first": true}))
        );
        response_tx.send(Ok(())).unwrap();
    });
    result.unwrap();
    assert!(second_rx.try_recv().is_err());
    let error = JSONRPCErrorError {
        code: -32000,
        message: "declined".into(),
        data: None,
    };
    let (result, ()) = tokio::join!(other.reject(error.clone()), async {
        let ClientCommand::RejectServerRequest {
            request_id,
            error: received,
            response_tx,
        } = second_rx.recv().await.unwrap()
        else {
            panic!("wrong command")
        };
        assert_eq!((request_id, received), (id, error));
        response_tx.send(Ok(())).unwrap();
    });
    result.unwrap();
    assert!(first_rx.try_recv().is_err());
    drop(first_rx);
    assert_eq!(
        stale
            .resolve(serde_json::json!({}))
            .await
            .unwrap_err()
            .kind(),
        io::ErrorKind::BrokenPipe
    );
    drop(other);
    assert!(second_rx.try_recv().is_err());
}

#[tokio::test]
async fn cloned_reply_handle_only_submits_once() {
    let (command_tx, mut command_rx) = mpsc::channel(/*buffer*/ 2);
    let client = AppServerRequestHandle::InProcess(InProcessAppServerRequestHandle { command_tx });
    let reply = client.server_request_reply_handle(RequestId::Integer(1));
    let duplicate = reply.clone();
    let error = JSONRPCErrorError {
        code: -32000,
        message: "declined".into(),
        data: None,
    };

    let (resolved, rejected, ()) = tokio::join!(
        reply.resolve(serde_json::json!({"accepted": true})),
        duplicate.reject(error),
        async {
            match command_rx.recv().await.unwrap() {
                ClientCommand::ResolveServerRequest { response_tx, .. }
                | ClientCommand::RejectServerRequest { response_tx, .. } => {
                    response_tx.send(Ok(())).unwrap();
                }
                _ => panic!("wrong command"),
            }
        }
    );
    let outcomes = [resolved, rejected].map(|result| result.map_err(|error| error.kind()));
    assert!(
        outcomes == [Ok(()), Err(io::ErrorKind::InvalidInput)]
            || outcomes == [Err(io::ErrorKind::InvalidInput), Ok(())]
    );
    assert!(command_rx.try_recv().is_err());
}
