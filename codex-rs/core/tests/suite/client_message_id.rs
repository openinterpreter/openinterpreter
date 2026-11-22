//! Turn input carrying a client message id is accepted at most once per thread.

use codex_core::SteerSubmission;
use codex_core::TurnInput;
use codex_core::TurnInputRequest;
use codex_core::TurnInputSubmission;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::Op;
use codex_protocol::user_input::UserInput;
use core_test_support::hooks::trust_discovered_hooks;
use core_test_support::responses;
use core_test_support::responses::ev_completed;
use core_test_support::responses::ev_response_created;
use core_test_support::streaming_sse::StreamingSseChunk;
use core_test_support::streaming_sse::start_streaming_sse_server;
use core_test_support::test_codex::test_codex;
use core_test_support::wait_for_event;
use pretty_assertions::assert_eq;
use serde_json::Value;
use std::time::Duration;
use tokio::sync::oneshot;
use tokio::time::timeout;

fn user_message(text: &str, client_id: Option<&str>) -> TurnInputRequest {
    TurnInputRequest::new(TurnInput::UserInput {
        content: vec![UserInput::Text {
            text: text.to_string(),
            text_elements: Vec::new(),
        }],
        client_id: client_id.map(str::to_string),
    })
}

/// User message texts in a raw model request, limited to `wanted` so injected
/// context does not obscure the comparison.
fn user_texts(request: &[u8], wanted: &[&str]) -> Vec<String> {
    let body: Value = serde_json::from_slice(request).expect("parse model request");
    body["input"]
        .as_array()
        .expect("model request input")
        .iter()
        .filter(|item| item["type"] == "message" && item["role"] == "user")
        .filter_map(|item| item["content"].as_array())
        .flatten()
        .filter_map(|span| span["text"].as_str())
        .filter(|text| wanted.contains(text))
        .map(str::to_string)
        .collect()
}

/// A first response held open by `gate`, followed by one that completes.
fn held_then_completed(gate: oneshot::Receiver<()>) -> Vec<Vec<StreamingSseChunk>> {
    vec![
        vec![
            StreamingSseChunk {
                gate: None,
                body: responses::sse(vec![ev_response_created("resp-1")]),
            },
            StreamingSseChunk {
                gate: Some(gate),
                body: responses::sse(vec![ev_completed("resp-1")]),
            },
        ],
        vec![StreamingSseChunk {
            gate: None,
            body: responses::sse_completed("resp-2"),
        }],
    ]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repeated_client_message_id_during_turn_adds_input_once() -> anyhow::Result<()> {
    let (release_response, response_gate) = oneshot::channel();
    let (server, _completions) =
        start_streaming_sse_server(held_then_completed(response_gate)).await;
    let test = test_codex()
        .with_model("gpt-5.4")
        .build_with_streaming_server(&server)
        .await?;
    let codex = &test.codex;

    let TurnInputSubmission::Started {
        turn_id,
        root_turn_id,
    } = codex
        .start_or_steer_turn(user_message("start prompt", Some("start-id")))
        .await?
    else {
        panic!("the first message should start a turn");
    };
    timeout(
        Duration::from_secs(5),
        server.wait_for_request_count(/*count*/ 1),
    )
    .await?;

    let steered = TurnInputSubmission::Steered {
        turn_id: turn_id.clone(),
        root_turn_id: root_turn_id.clone(),
    };
    assert_eq!(
        codex
            .start_or_steer_turn(user_message("start prompt", Some("start-id")))
            .await?,
        steered
    );
    for _ in 0..2 {
        assert_eq!(
            codex
                .steer_turn(
                    user_message("steer prompt", Some("steer-id")),
                    turn_id.clone()
                )
                .await?,
            SteerSubmission::Steered {
                turn_id: turn_id.clone()
            }
        );
        // Input without an id is never deduplicated.
        assert_eq!(
            codex
                .start_or_steer_turn(user_message("unidentified prompt", /*client_id*/ None))
                .await?,
            steered
        );
    }
    assert_eq!(
        codex
            .start_or_steer_turn(user_message("steer prompt", Some("steer-id")))
            .await?,
        steered
    );

    release_response
        .send(())
        .expect("response gate should remain open");
    wait_for_event(codex, |event| matches!(event, EventMsg::TurnComplete(_))).await;

    let requests = server.requests().await;
    assert_eq!(requests.len(), 2);
    assert_eq!(
        user_texts(
            &requests[1],
            &["start prompt", "steer prompt", "unidentified prompt"]
        ),
        vec![
            "start prompt",
            "steer prompt",
            "unidentified prompt",
            "unidentified prompt"
        ]
    );
    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn repeated_client_message_id_after_turn_is_no_op() -> anyhow::Result<()> {
    let server = responses::start_mock_server().await;
    let mock = responses::mount_sse_sequence(
        &server,
        vec![
            responses::sse_completed("resp-1"),
            responses::sse_completed("resp-2"),
        ],
    )
    .await;
    let test = test_codex().build_with_auto_env(&server).await?;
    let codex = &test.codex;

    let TurnInputSubmission::Started {
        turn_id,
        root_turn_id,
    } = codex
        .start_or_steer_turn(user_message("only once", Some("idle-id")))
        .await?
    else {
        panic!("the first message should start a turn");
    };
    wait_for_event(codex, |event| matches!(event, EventMsg::TurnComplete(_))).await;

    assert_eq!(
        codex
            .start_or_steer_turn(user_message("only once", Some("idle-id")))
            .await?,
        TurnInputSubmission::Steered {
            turn_id: turn_id.clone(),
            root_turn_id,
        }
    );
    // A repeat names a stale turn, but the original request already succeeded.
    assert_eq!(
        codex
            .steer_turn(
                user_message("only once", Some("idle-id")),
                "stale-turn".to_string()
            )
            .await?,
        SteerSubmission::Steered { turn_id }
    );

    codex
        .start_or_steer_turn(user_message("next prompt", /*client_id*/ None))
        .await?;
    wait_for_event(codex, |event| matches!(event, EventMsg::TurnComplete(_))).await;
    let requests = mock.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[1]
            .message_input_texts("user")
            .into_iter()
            .filter(|text| text == "only once" || text == "next prompt")
            .collect::<Vec<_>>(),
        vec!["only once", "next prompt"]
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interrupt_discards_pending_input_so_retry_is_accepted() -> anyhow::Result<()> {
    let (hold_first_response, first_gate) = oneshot::channel::<()>();
    let (server, _completions) = start_streaming_sse_server(held_then_completed(first_gate)).await;
    let test = test_codex()
        .with_model("gpt-5.4")
        .build_with_streaming_server(&server)
        .await?;
    let codex = &test.codex;

    let TurnInputSubmission::Started {
        turn_id: interrupted_turn,
        ..
    } = codex
        .start_or_steer_turn(user_message("first prompt", /*client_id*/ None))
        .await?
    else {
        panic!("the first message should start a turn");
    };
    timeout(
        Duration::from_secs(5),
        server.wait_for_request_count(/*count*/ 1),
    )
    .await?;
    assert_eq!(
        codex
            .steer_turn(
                user_message("retry me", Some("retry-id")),
                interrupted_turn.clone()
            )
            .await?,
        SteerSubmission::Steered {
            turn_id: interrupted_turn.clone()
        }
    );
    codex.submit(Op::Interrupt).await?;
    wait_for_event(codex, |event| matches!(event, EventMsg::TurnAborted(_))).await;

    let retried = codex
        .start_or_steer_turn(user_message("retry me", Some("retry-id")))
        .await?;
    let TurnInputSubmission::Started { turn_id, .. } = &retried else {
        panic!("a retry of discarded input should start a turn: {retried:?}");
    };
    assert_ne!(turn_id, &interrupted_turn);
    wait_for_event(codex, |event| matches!(event, EventMsg::TurnComplete(_))).await;

    let requests = server.requests().await;
    assert_eq!(requests.len(), 2);
    assert_eq!(
        user_texts(&requests[1], &["first prompt", "retry me"]),
        vec!["first prompt", "retry me"]
    );
    drop(hold_first_response);
    server.shutdown().await;
    Ok(())
}

/// A SessionStart hook that stops the first turn ends it before its starting
/// input is recorded, so a retry with the same id starts a normal turn.
#[tokio::test]
async fn input_dropped_before_recording_is_accepted_on_retry() -> anyhow::Result<()> {
    let server = responses::start_mock_server().await;
    let mock = responses::mount_sse_once(&server, responses::sse_completed("resp-1")).await;
    let mut builder = test_codex()
        .with_pre_build_hook(|home| {
            let script_path = home.join("stop_session_start.py");
            std::fs::write(
                &script_path,
                r#"print('{"continue": false, "stopReason": "not yet"}')"#,
            )
            .expect("write session start hook script");
            let hooks = serde_json::json!({
                "hooks": {
                    "SessionStart": [{
                        "hooks": [{
                            "type": "command",
                            "command": format!("python3 {}", script_path.display()),
                        }]
                    }]
                }
            });
            std::fs::write(home.join("hooks.json"), hooks.to_string()).expect("write hooks.json");
        })
        .with_config(trust_discovered_hooks);
    let test = builder.build(&server).await?;
    let codex = &test.codex;

    let TurnInputSubmission::Started {
        turn_id: stopped_turn,
        ..
    } = codex
        .start_or_steer_turn(user_message("retry me", Some("retry-id")))
        .await?
    else {
        panic!("the first message should start a turn");
    };
    wait_for_event(codex, |event| matches!(event, EventMsg::TurnComplete(_))).await;

    let retried = codex
        .start_or_steer_turn(user_message("retry me", Some("retry-id")))
        .await?;
    let TurnInputSubmission::Started { turn_id, .. } = &retried else {
        panic!("a retry of input that was never recorded should start a turn: {retried:?}");
    };
    assert_ne!(turn_id, &stopped_turn);
    wait_for_event(codex, |event| matches!(event, EventMsg::TurnComplete(_))).await;

    assert_eq!(
        mock.single_request()
            .message_input_texts("user")
            .into_iter()
            .filter(|text| text == "retry me")
            .collect::<Vec<_>>(),
        vec!["retry me"]
    );
    Ok(())
}
