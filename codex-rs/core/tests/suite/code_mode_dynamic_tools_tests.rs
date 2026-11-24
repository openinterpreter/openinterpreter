//! Dynamic tool calls remain independent while waiting for client responses.

use super::*;
use pretty_assertions::assert_eq;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn code_mode_dynamic_tool_calls_pending_together_resolve_in_reverse_order() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = responses::start_mock_server().await;
    let mut builder = test_codex()
        .with_model_info_override("gpt-5.5", |model_info| {
            model_info.use_responses_lite = true;
            model_info.tool_mode = Some(ToolMode::CodeMode);
        })
        .with_config(|config| {
            config.features.enable(Feature::CodeMode).unwrap();
        });
    let mut test = builder.build_with_auto_env(&server).await?;
    let new_thread = test
        .thread_manager
        .start_thread(StartThreadOptions {
            dynamic_tools: vec![DynamicToolSpec::Function(DynamicToolFunctionSpec {
                name: "lookup".to_string(),
                description: "Look up a value by key.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": { "key": { "type": "string" } },
                    "required": ["key"],
                    "additionalProperties": false,
                }),
                defer_loading: false,
            })],
            ..StartThreadOptions::new(test.config.clone())
        })
        .await?;
    test.codex = new_thread.thread;
    test.session_configured = new_thread.session_configured;

    responses::mount_sse_once(
        &server,
        sse(vec![
            ev_response_created("resp-1"),
            ev_custom_tool_call(
                "call-1",
                "exec",
                r#"
const results = await Promise.all([
  tools.lookup({ key: "A" }),
  tools.lookup({ key: "B" }),
]);
text(JSON.stringify(results));
"#,
            ),
            ev_completed("resp-1"),
        ]),
    )
    .await;
    let second_mock = responses::mount_sse_once(
        &server,
        sse(vec![
            ev_assistant_message("msg-1", "done"),
            ev_completed("resp-2"),
        ]),
    )
    .await;

    test.codex
        .start_or_steer_turn(TurnInputRequest::user_input(vec![UserInput::Text {
            text: "Look up A and B concurrently.".to_string(),
            text_elements: Vec::new(),
        }]))
        .await?;

    // Neither response is submitted until both calls are waiting for the client.
    let mut pending = Vec::new();
    for _ in 0..2 {
        pending.push(
            wait_for_event_match(&test.codex, |event| match event {
                EventMsg::DynamicToolCallRequest(request) => Some(request.clone()),
                _ => None,
            })
            .await,
        );
    }
    assert_ne!(pending[0].call_id, pending[1].call_id);

    for request in pending.into_iter().rev() {
        let key = request.arguments["key"].as_str().unwrap();
        test.codex
            .submit(Op::DynamicToolResponse {
                id: request.call_id,
                response: DynamicToolResponse {
                    content_items: vec![DynamicToolCallOutputContentItem::InputText {
                        text: format!("value-{key}"),
                    }],
                    success: true,
                },
            })
            .await?;
    }
    wait_for_event(&test.codex, |event| {
        matches!(event, EventMsg::TurnComplete(_))
    })
    .await;

    let request = second_mock.single_request();
    let output = custom_tool_output_last_non_empty_text(&request, "call-1")
        .expect("the completed program should emit its results");
    assert_eq!(
        serde_json::from_str::<Value>(&output)?,
        serde_json::json!(["value-A", "value-B"])
    );
    Ok(())
}
