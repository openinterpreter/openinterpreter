use super::*;
use crate::ThreadManager;
use crate::agent::claude_agent_external_id;
use crate::session::tests::make_session_and_context;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::registry::ToolHandler;
use crate::turn_diff_tracker::TurnDiffTracker;
use codex_login::CodexAuth;
use codex_model_provider_info::built_in_model_providers;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::TurnCompleteEvent;
use pretty_assertions::assert_eq;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::timeout;

fn invocation(
    session: Arc<crate::session::Session>,
    turn: Arc<crate::session::TurnContext>,
    arguments: serde_json::Value,
) -> ToolInvocation {
    ToolInvocation {
        session,
        turn,
        tracker: Arc::new(Mutex::new(TurnDiffTracker::default())),
        call_id: "call_1".to_string(),
        tool_name: codex_tools::ToolName::plain("Agent"),
        payload: ToolPayload::Function {
            arguments: arguments.to_string(),
        },
    }
}

fn thread_manager() -> ThreadManager {
    ThreadManager::with_models_provider_for_tests(
        CodexAuth::from_api_key("dummy"),
        built_in_model_providers(/*openai_base_url*/ None)["openai"].clone(),
    )
}

#[tokio::test]
async fn foreground_agent_returns_child_completion_message() {
    let (mut session, turn) = make_session_and_context().await;
    let manager = thread_manager();
    let root = manager
        .start_thread((*turn.config).clone())
        .await
        .expect("root thread should start");
    session.services.agent_control = manager.agent_control();
    session.conversation_id = root.thread_id;
    let session = Arc::new(session);
    let turn = Arc::new(turn);

    let handle = tokio::spawn({
        let session = session.clone();
        let turn = turn.clone();
        async move {
            ClaudeAgentHandler
                .handle(invocation(
                    session,
                    turn,
                    json!({
                        "description": "Child task",
                        "prompt": "Reply with CHILD_DONE and nothing else."
                    }),
                ))
                .await
        }
    });

    let child_thread_id = timeout(Duration::from_secs(5), async {
        loop {
            let mut subtree = manager
                .agent_control()
                .list_live_agent_subtree_thread_ids(root.thread_id)
                .await
                .expect("subtree should load");
            subtree.retain(|thread_id| *thread_id != root.thread_id);
            if let Some(child_thread_id) = subtree.into_iter().next() {
                break child_thread_id;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("child should spawn");

    let child_thread = manager
        .get_thread(child_thread_id)
        .await
        .expect("child thread should exist");
    let child_turn = child_thread.codex.session.new_default_turn().await;
    child_thread
        .codex
        .session
        .send_event(
            child_turn.as_ref(),
            EventMsg::TurnComplete(TurnCompleteEvent {
                turn_id: child_turn.sub_id.clone(),
                last_agent_message: Some("CHILD_DONE".to_string()),
                completed_at: None,
                duration_ms: None,
            }),
        )
        .await;

    let output = handle
        .await
        .expect("join should succeed")
        .expect("Agent should succeed");
    assert_eq!(
        output.body.first(),
        Some(
            &codex_protocol::models::FunctionCallOutputContentItem::InputText {
                text: "CHILD_DONE".to_string()
            }
        )
    );
    let footer = match output.body.get(1) {
        Some(codex_protocol::models::FunctionCallOutputContentItem::InputText { text }) => text,
        other => panic!("expected structured footer text item, got {other:?}"),
    };
    let child_external_id = claude_agent_external_id(child_thread_id);
    assert!(footer.contains(&format!(
        "agentId: {child_external_id} (use SendMessage with to: '{child_external_id}' to continue this agent)"
    )));
    assert!(footer.contains("<usage>total_tokens: 0\ntool_uses: 0\nduration_ms: "));
    assert!(footer.ends_with("</usage>"));
}

#[test]
fn claude_agent_prompt_uses_raw_task_prompt_by_default() {
    assert_eq!(
        build_claude_agent_prompt(
            "Child proof",
            "Create child-proof.txt and reply CHILD_DONE.",
            None,
        ),
        "Create child-proof.txt and reply CHILD_DONE."
    );
}

#[test]
fn foreground_output_includes_structured_agent_usage_footer() {
    let output = build_claude_agent_foreground_output(
        "CHILD_DONE".to_string(),
        &ClaudeAgentForegroundResultMetadata {
            agent_id: "agent_123".to_string(),
            total_tokens: Some(321),
            observable_usage: Some(AgentObservableUsage {
                tool_uses: 2,
                duration_ms: Some(4567),
            }),
            wait_duration_ms: 9999,
        },
    );

    assert_eq!(
        output.body,
        vec![
            codex_protocol::models::FunctionCallOutputContentItem::InputText {
                text: "CHILD_DONE".to_string()
            },
            codex_protocol::models::FunctionCallOutputContentItem::InputText {
                text: "agentId: agent_123 (use SendMessage with to: 'agent_123' to continue this agent)\n<usage>total_tokens: 321\ntool_uses: 2\nduration_ms: 4567</usage>".to_string()
            }
        ]
    );
}
