use anyhow::Result;
use app_test_support::MockResponsesConfig;
use app_test_support::TestAppServer;
use app_test_support::create_mock_responses_server_repeating_assistant;
use codex_app_server_protocol::ClientRequest;
use codex_app_server_protocol::ThreadHistoryMode;
use codex_app_server_protocol::ThreadItemPayload;
use codex_app_server_protocol::ThreadReadParams;
use codex_app_server_protocol::ThreadReadResponse;
use codex_app_server_protocol::ThreadResumeParams;
use codex_app_server_protocol::ThreadResumeResponse;
use codex_app_server_protocol::ThreadStartParams;
use codex_app_server_protocol::ThreadStartResponse;
use codex_app_server_protocol::TurnStartParams;
use codex_app_server_protocol::TurnStartResponse;
use codex_app_server_protocol::TurnSteerParams;
use codex_app_server_protocol::TurnSteerResponse;
use codex_app_server_protocol::UserInput;
use pretty_assertions::assert_eq;
use tempfile::TempDir;
use tokio::time::timeout;

#[cfg(windows)]
const DEFAULT_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(25);
#[cfg(not(windows))]
const DEFAULT_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

const CLIENT_MESSAGE_ID: &str = "client-message-1";

fn input() -> Vec<UserInput> {
    vec![UserInput::Text {
        text: "say hello once".to_string(),
        text_elements: Vec::new(),
    }]
}

fn turn_start(thread_id: &str) -> TurnStartParams {
    TurnStartParams {
        thread_id: thread_id.to_string(),
        client_user_message_id: Some(CLIENT_MESSAGE_ID.to_string()),
        input: input(),
        ..Default::default()
    }
}

/// Client ids of every user message recorded in the thread's history.
async fn recorded_client_ids(
    mcp: &mut TestAppServer,
    thread_id: &str,
) -> Result<Vec<Option<String>>> {
    let read_id = mcp
        .send_thread_read_request(ThreadReadParams {
            thread_id: thread_id.to_string(),
            include_turns: true,
        })
        .await?;
    let ThreadReadResponse { thread, .. } =
        timeout(DEFAULT_READ_TIMEOUT, mcp.read_response(read_id)).await??;
    Ok(thread
        .turns
        .into_iter()
        .flat_map(|turn| turn.items)
        .filter_map(|item| match item.payload {
            ThreadItemPayload::UserMessage { client_id, .. } => Some(client_id),
            _ => None,
        })
        .collect())
}

#[tokio::test]
async fn repeated_client_message_id_returns_original_turn_across_resume() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    MockResponsesConfig::new(&server.uri())
        .with_model("gpt-5.4")
        .write(codex_home.path())?;

    let (thread_id, turn_id) = {
        let mut mcp = TestAppServer::builder()
            .with_codex_home(codex_home.path())
            .build_initialized()
            .await?;
        let start_id = mcp
            .send_thread_start_request_with_auto_env(ThreadStartParams {
                model: Some("gpt-5.4".to_string()),
                history_mode: Some(ThreadHistoryMode::Legacy),
                ..Default::default()
            })
            .await?;
        let ThreadStartResponse { thread, .. } =
            timeout(DEFAULT_READ_TIMEOUT, mcp.read_response(start_id)).await??;

        let TurnStartResponse { turn } = mcp
            .request(|request_id| ClientRequest::TurnStart {
                request_id,
                params: turn_start(&thread.id),
            })
            .await?;
        timeout(
            DEFAULT_READ_TIMEOUT,
            mcp.read_stream_until_notification_message("turn/completed"),
        )
        .await??;

        let TurnStartResponse { turn: repeated } = mcp
            .request(|request_id| ClientRequest::TurnStart {
                request_id,
                params: turn_start(&thread.id),
            })
            .await?;
        assert_eq!(repeated.id, turn.id);
        // The repeat names a stale turn, but the original request already succeeded.
        let steer: TurnSteerResponse = mcp
            .request(|request_id| ClientRequest::TurnSteer {
                request_id,
                params: TurnSteerParams {
                    thread_id: thread.id.clone(),
                    client_user_message_id: Some(CLIENT_MESSAGE_ID.to_string()),
                    input: input(),
                    responsesapi_client_metadata: None,
                    responsesapi_client_metadata_patch: None,
                    additional_context: None,
                    expected_turn_id: "stale-turn".to_string(),
                },
            })
            .await?;
        assert_eq!(steer.turn_id, turn.id);
        assert_eq!(
            recorded_client_ids(&mut mcp, &thread.id).await?,
            vec![Some(CLIENT_MESSAGE_ID.to_string())]
        );
        timeout(DEFAULT_READ_TIMEOUT, mcp.shutdown_gracefully()).await??;
        (thread.id, turn.id)
    };

    let mut mcp = TestAppServer::builder()
        .with_codex_home(codex_home.path())
        .build_initialized()
        .await?;
    let resume_id = mcp
        .send_thread_resume_request(ThreadResumeParams {
            thread_id: thread_id.clone(),
            ..Default::default()
        })
        .await?;
    let _: ThreadResumeResponse =
        timeout(DEFAULT_READ_TIMEOUT, mcp.read_response(resume_id)).await??;

    let TurnStartResponse { turn: repeated } = mcp
        .request(|request_id| ClientRequest::TurnStart {
            request_id,
            params: turn_start(&thread_id),
        })
        .await?;
    assert_eq!(repeated.id, turn_id);
    assert_eq!(
        recorded_client_ids(&mut mcp, &thread_id).await?,
        vec![Some(CLIENT_MESSAGE_ID.to_string())]
    );
    Ok(())
}
