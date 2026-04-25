use crate::agent::AgentStatus;
use crate::agent::claude_agent_external_id;
use crate::agent::control::AgentObservableUsage;
use crate::agent::exceeds_thread_spawn_depth_limit;
use crate::agent::next_thread_spawn_depth;
use crate::agent::role::apply_role_to_config;
use crate::function_tool::FunctionCallError;
use crate::session::Session;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::multi_agents_common::apply_requested_spawn_agent_model_overrides;
use crate::tools::handlers::multi_agents_common::apply_spawn_agent_overrides;
use crate::tools::handlers::multi_agents_common::apply_spawn_agent_runtime_overrides;
use crate::tools::handlers::multi_agents_common::build_agent_spawn_config;
use crate::tools::handlers::multi_agents_common::collab_spawn_error;
use crate::tools::handlers::multi_agents_common::thread_spawn_source;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;
use codex_models_manager::manager::RefreshStrategy;
use codex_protocol::models::DeveloperInstructions;
use codex_protocol::models::FunctionCallOutputContentItem;
use codex_protocol::protocol::AgentStatus as ProtocolAgentStatus;
use serde::Deserialize;
use serde::Serialize;
use std::time::Instant;

pub struct ClaudeAgentHandler;

const CLAUDE_AGENT_EMPTY_OUTPUT: &str = "(Agent completed with no output)";
const CLAUDE_AGENT_DEVELOPER_INSTRUCTIONS: &str = r#"<spawned_agent_context>
You are a newly spawned agent in a team of agents collaborating to complete a task. You can spawn sub-agents to handle subtasks, and those sub-agents can spawn their own sub-agents. You are responsible for returning the response to your assigned task in the final channel. When you give your response, the contents of your response in the final channel will be immediately delivered back to your parent agent. The prior conversation history was forked from your parent agent. Treat the next user message as your assigned task, and use the forked history only as background context.
</spawned_agent_context>"#;

impl ToolHandler for ClaudeAgentHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolInvocation {
            session,
            turn,
            payload,
            ..
        } = invocation;
        let ToolPayload::Function { arguments } = payload else {
            return Err(FunctionCallError::RespondToModel(
                "Agent received unsupported payload".to_string(),
            ));
        };
        let args: ClaudeAgentArgs = parse_arguments(&arguments)?;
        if args.description.trim().is_empty() || args.prompt.trim().is_empty() {
            return Err(FunctionCallError::RespondToModel(
                "Agent requires non-empty description and prompt".to_string(),
            ));
        }
        if args.isolation.is_some() {
            return Err(FunctionCallError::RespondToModel(
                "Agent isolation=worktree is not implemented for the claude-code harness yet."
                    .to_string(),
            ));
        }

        let child_depth = next_thread_spawn_depth(&turn.session_source);
        if exceeds_thread_spawn_depth_limit(child_depth, turn.config.agent_max_depth) {
            return Err(FunctionCallError::RespondToModel(
                "Agent depth limit reached. Solve the task yourself.".to_string(),
            ));
        }

        let (role_name, prompt_prefix) = claude_agent_profile(args.subagent_type.as_deref());
        let mut config =
            build_agent_spawn_config(&session.get_base_instructions().await, turn.as_ref())?;
        let resolved_model =
            resolve_requested_claude_agent_model(session.as_ref(), args.model.as_ref()).await?;
        apply_requested_spawn_agent_model_overrides(
            &session,
            turn.as_ref(),
            &mut config,
            resolved_model.as_deref(),
            /*requested_reasoning_effort*/ None,
        )
        .await?;
        if let Some(role_name) = role_name {
            apply_role_to_config(&mut config, Some(role_name))
                .await
                .map_err(FunctionCallError::RespondToModel)?;
        }
        apply_spawn_agent_runtime_overrides(&mut config, turn.as_ref())?;
        apply_spawn_agent_overrides(&mut config, child_depth);
        config.developer_instructions = Some(
            if let Some(existing_instructions) = config.developer_instructions.take() {
                DeveloperInstructions::new(existing_instructions)
                    .concat(DeveloperInstructions::new(
                        CLAUDE_AGENT_DEVELOPER_INSTRUCTIONS,
                    ))
                    .into_text()
            } else {
                DeveloperInstructions::new(CLAUDE_AGENT_DEVELOPER_INSTRUCTIONS).into_text()
            },
        );

        let initial_prompt =
            build_claude_agent_prompt(args.description.trim(), args.prompt.trim(), prompt_prefix);
        let spawn_source = thread_spawn_source(
            session.conversation_id,
            &turn.session_source,
            child_depth,
            role_name,
            /*task_name*/ None,
        )?;
        let spawned_agent = session
            .services
            .agent_control
            .spawn_agent_with_metadata(
                config,
                vec![codex_protocol::user_input::UserInput::Text {
                    text: initial_prompt,
                    text_elements: Vec::new(),
                }]
                .into(),
                Some(spawn_source),
                Default::default(),
            )
            .await
            .map_err(collab_spawn_error)?;

        if args.run_in_background.unwrap_or(false) {
            let content = serde_json::to_string(&ClaudeBackgroundAgentResult {
                agent_id: claude_agent_external_id(spawned_agent.thread_id),
                nickname: spawned_agent.metadata.agent_nickname,
            })
            .map_err(|err| {
                FunctionCallError::Fatal(format!(
                    "failed to serialize background Agent result: {err}"
                ))
            })?;
            return Ok(FunctionToolOutput::from_text(content, Some(true)));
        }

        let wait_started_at = Instant::now();
        let status = wait_for_claude_agent_completion(&session, spawned_agent.thread_id).await?;
        let response_text = match status {
            ProtocolAgentStatus::Completed(Some(message)) => message,
            ProtocolAgentStatus::Completed(None) => CLAUDE_AGENT_EMPTY_OUTPUT.to_string(),
            ProtocolAgentStatus::Errored(message) => {
                return Err(FunctionCallError::RespondToModel(format!(
                    "Agent failed: {message}"
                )));
            }
            ProtocolAgentStatus::Interrupted => {
                return Err(FunctionCallError::RespondToModel(
                    "Agent was interrupted before it completed.".to_string(),
                ));
            }
            ProtocolAgentStatus::Shutdown => {
                return Err(FunctionCallError::RespondToModel(
                    "Agent shut down before it completed.".to_string(),
                ));
            }
            ProtocolAgentStatus::NotFound => {
                return Err(FunctionCallError::RespondToModel(
                    "Agent disappeared before it completed.".to_string(),
                ));
            }
            ProtocolAgentStatus::PendingInit | ProtocolAgentStatus::Running => {
                return Err(FunctionCallError::RespondToModel(
                    "Agent did not reach a final state.".to_string(),
                ));
            }
        };

        let observable_usage = session
            .services
            .agent_control
            .get_observable_usage(spawned_agent.thread_id)
            .await;
        let total_tokens = session
            .services
            .agent_control
            .get_last_token_usage(spawned_agent.thread_id)
            .await
            .map(|usage| usage.total_tokens);
        let metadata = ClaudeAgentForegroundResultMetadata {
            agent_id: claude_agent_external_id(spawned_agent.thread_id),
            total_tokens,
            observable_usage,
            wait_duration_ms: wait_started_at.elapsed().as_millis() as i64,
        };

        Ok(build_claude_agent_foreground_output(
            response_text,
            &metadata,
        ))
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ClaudeAgentArgs {
    description: String,
    prompt: String,
    subagent_type: Option<String>,
    model: Option<ClaudeAgentModel>,
    run_in_background: Option<bool>,
    isolation: Option<ClaudeAgentIsolation>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ClaudeAgentModel {
    Sonnet,
    Opus,
    Haiku,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ClaudeAgentIsolation {
    Worktree,
}

#[derive(Debug, Serialize)]
struct ClaudeBackgroundAgentResult {
    agent_id: String,
    nickname: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ClaudeAgentForegroundResultMetadata {
    agent_id: String,
    total_tokens: Option<i64>,
    observable_usage: Option<AgentObservableUsage>,
    wait_duration_ms: i64,
}

fn build_claude_agent_prompt(
    _description: &str,
    prompt: &str,
    prompt_prefix: Option<&'static str>,
) -> String {
    match prompt_prefix {
        Some(prefix) => format!("{prefix}\n\n{prompt}"),
        None => prompt.to_string(),
    }
}

fn claude_agent_profile(
    subagent_type: Option<&str>,
) -> (Option<&'static str>, Option<&'static str>) {
    match subagent_type.map(str::trim) {
        Some("Explore") => (Some("explorer"), None),
        Some("Plan") => (
            None,
            Some(
                "Approach this as a planning specialist. Focus on implementation strategy, critical files, and trade-offs before recommending concrete next steps.",
            ),
        ),
        Some("statusline-setup") => (
            None,
            Some(
                "Focus narrowly on configuring the Claude Code status line setting and any directly related files.",
            ),
        ),
        _ => (None, None),
    }
}

fn build_claude_agent_foreground_output(
    response_text: String,
    metadata: &ClaudeAgentForegroundResultMetadata,
) -> FunctionToolOutput {
    let tool_uses = metadata
        .observable_usage
        .map(|usage| usage.tool_uses)
        .unwrap_or(0);
    let total_tokens = metadata.total_tokens.unwrap_or(0);
    let duration_ms = metadata
        .observable_usage
        .and_then(|usage| usage.duration_ms)
        .unwrap_or(metadata.wait_duration_ms);
    let usage_text = format!(
        "agentId: {} (use SendMessage with to: '{}' to continue this agent)\n<usage>total_tokens: {total_tokens}\ntool_uses: {tool_uses}\nduration_ms: {duration_ms}</usage>",
        metadata.agent_id, metadata.agent_id,
    );

    FunctionToolOutput::from_content(
        vec![
            FunctionCallOutputContentItem::InputText {
                text: response_text,
            },
            FunctionCallOutputContentItem::InputText { text: usage_text },
        ],
        Some(true),
    )
}

async fn resolve_requested_claude_agent_model(
    session: &Session,
    requested_model: Option<&ClaudeAgentModel>,
) -> Result<Option<String>, FunctionCallError> {
    let Some(requested_model) = requested_model else {
        return Ok(None);
    };

    let family_prefix = match requested_model {
        ClaudeAgentModel::Sonnet => "claude-sonnet",
        ClaudeAgentModel::Opus => "claude-opus",
        ClaudeAgentModel::Haiku => "claude-haiku",
    };
    let mut candidates = session
        .services
        .models_manager
        .list_models(RefreshStrategy::Offline)
        .await
        .into_iter()
        .filter(|model| model.supported_in_api && model.show_in_picker)
        .filter(|model| model.model.contains(family_prefix))
        .map(|model| model.model)
        .collect::<Vec<_>>();
    candidates.sort();
    candidates.reverse();
    candidates.into_iter().next().map(Some).ok_or_else(|| {
        FunctionCallError::RespondToModel(format!(
            "No picker-ready Claude model was available for `{family_prefix}`."
        ))
    })
}

async fn wait_for_claude_agent_completion(
    session: &Session,
    agent_id: codex_protocol::ThreadId,
) -> Result<ProtocolAgentStatus, FunctionCallError> {
    let mut status_rx = session
        .services
        .agent_control
        .subscribe_status(agent_id)
        .await
        .map_err(|err| FunctionCallError::RespondToModel(format!("Agent failed: {err}")))?;
    let mut status = status_rx.borrow().clone();
    while !is_final_agent_status(&status) {
        if status_rx.changed().await.is_err() {
            return Ok(session.services.agent_control.get_status(agent_id).await);
        }
        status = status_rx.borrow().clone();
    }
    Ok(status)
}

fn is_final_agent_status(status: &ProtocolAgentStatus) -> bool {
    !matches!(
        status,
        AgentStatus::PendingInit | AgentStatus::Running | AgentStatus::Interrupted
    )
}

#[cfg(test)]
#[path = "claude_code_agent_tests.rs"]
mod tests;
