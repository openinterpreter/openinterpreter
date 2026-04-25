use crate::agent::AgentStatus;
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
use crate::tools::handlers::multi_agents_common::build_agent_resume_config;
use crate::tools::handlers::multi_agents_common::build_agent_spawn_config;
use crate::tools::handlers::multi_agents_common::collab_agent_error;
use crate::tools::handlers::multi_agents_common::collab_spawn_error;
use crate::tools::handlers::multi_agents_common::thread_spawn_source;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;
use codex_protocol::ThreadId;
use codex_protocol::protocol::AgentStatus as ProtocolAgentStatus;
use codex_protocol::user_input::UserInput;
use serde::Deserialize;
use serde::Serialize;
use std::time::Duration;
use tokio::time::timeout;

pub struct KimiAgentHandler;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct KimiAgentArgs {
    description: String,
    prompt: String,
    subagent_type: Option<String>,
    model: Option<String>,
    resume: Option<String>,
    run_in_background: Option<bool>,
    timeout: Option<u64>,
}

#[derive(Debug, Serialize)]
struct KimiBackgroundAgentResult {
    task_id: String,
    status: String,
    description: String,
}

impl ToolHandler for KimiAgentHandler {
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
        let args: KimiAgentArgs = parse_arguments(&arguments)?;
        if args.description.trim().is_empty() || args.prompt.trim().is_empty() {
            return Err(FunctionCallError::RespondToModel(
                "Agent requires non-empty description and prompt".to_string(),
            ));
        }
        let child_depth = next_thread_spawn_depth(&turn.session_source);
        if exceeds_thread_spawn_depth_limit(child_depth, turn.config.agent_max_depth) {
            return Err(FunctionCallError::RespondToModel(
                "Agent depth limit reached. Solve the task yourself.".to_string(),
            ));
        }

        let (agent_role, prompt_prefix) = kimi_agent_profile(args.subagent_type.as_deref());
        let initial_prompt = build_kimi_agent_prompt(args.prompt.trim(), prompt_prefix);
        let timeout_duration = args.timeout.map(Duration::from_secs);

        let agent_id = if let Some(resume) = args.resume.as_deref() {
            if args.model.is_some() {
                return Err(FunctionCallError::RespondToModel(
                    "Agent resume does not support overriding model in the kimi-cli harness yet."
                        .to_string(),
                ));
            }
            send_input_to_resumed_agent(
                session.as_ref(),
                turn.as_ref(),
                &initial_prompt,
                resume,
                child_depth,
            )
            .await?
        } else {
            let mut config =
                build_agent_spawn_config(&session.get_base_instructions().await, turn.as_ref())?;
            apply_requested_spawn_agent_model_overrides(
                &session,
                turn.as_ref(),
                &mut config,
                args.model.as_deref(),
                /*requested_reasoning_effort*/ None,
            )
            .await?;
            if let Some(agent_role) = agent_role {
                apply_role_to_config(&mut config, Some(agent_role))
                    .await
                    .map_err(FunctionCallError::RespondToModel)?;
            }
            apply_spawn_agent_runtime_overrides(&mut config, turn.as_ref())?;
            apply_spawn_agent_overrides(&mut config, child_depth);
            let spawn_source = thread_spawn_source(
                session.conversation_id,
                &turn.session_source,
                child_depth,
                agent_role,
                /*task_name*/ None,
            )?;
            session
                .services
                .agent_control
                .spawn_agent_with_metadata(
                    config,
                    vec![UserInput::Text {
                        text: initial_prompt,
                        text_elements: Vec::new(),
                    }]
                    .into(),
                    Some(spawn_source),
                    Default::default(),
                )
                .await
                .map_err(collab_spawn_error)?
                .thread_id
        };

        if args.run_in_background.unwrap_or(false) {
            let result = KimiBackgroundAgentResult {
                task_id: agent_id.to_string(),
                status: "running".to_string(),
                description: args.description,
            };
            let output = serde_json::to_string(&result).map_err(|err| {
                FunctionCallError::Fatal(format!(
                    "failed to serialize background Agent result: {err}"
                ))
            })?;
            return Ok(FunctionToolOutput::from_text(output, Some(true)));
        }

        let status = if let Some(timeout_duration) = timeout_duration {
            timeout(
                timeout_duration,
                wait_for_kimi_agent_completion(session.as_ref(), agent_id),
            )
            .await
            .map_err(|_| {
                FunctionCallError::RespondToModel(format!(
                    "Agent timed out after {}s.",
                    timeout_duration.as_secs()
                ))
            })??
        } else {
            wait_for_kimi_agent_completion(session.as_ref(), agent_id).await?
        };

        match status {
            ProtocolAgentStatus::Completed(Some(message)) => {
                Ok(FunctionToolOutput::from_text(message, Some(true)))
            }
            ProtocolAgentStatus::Completed(None) => Ok(FunctionToolOutput::from_text(
                "(Agent completed with no output)".to_string(),
                Some(true),
            )),
            ProtocolAgentStatus::Errored(message) => Err(FunctionCallError::RespondToModel(
                format!("Agent failed: {message}"),
            )),
            ProtocolAgentStatus::Interrupted => Err(FunctionCallError::RespondToModel(
                "Agent was interrupted before it completed.".to_string(),
            )),
            ProtocolAgentStatus::Shutdown => Err(FunctionCallError::RespondToModel(
                "Agent shut down before it completed.".to_string(),
            )),
            ProtocolAgentStatus::NotFound => Err(FunctionCallError::RespondToModel(
                "Agent disappeared before it completed.".to_string(),
            )),
            ProtocolAgentStatus::PendingInit | ProtocolAgentStatus::Running => Err(
                FunctionCallError::RespondToModel("Agent did not reach a final state.".to_string()),
            ),
        }
    }
}

async fn send_input_to_resumed_agent(
    session: &Session,
    turn: &crate::session::TurnContext,
    prompt: &str,
    resume: &str,
    child_depth: i32,
) -> Result<ThreadId, FunctionCallError> {
    let receiver_thread_id = ThreadId::from_string(resume).map_err(|err| {
        FunctionCallError::RespondToModel(format!("invalid agent id {resume}: {err:?}"))
    })?;
    if matches!(
        session
            .services
            .agent_control
            .get_status(receiver_thread_id)
            .await,
        AgentStatus::NotFound
    ) {
        let config = build_agent_resume_config(turn, child_depth)?;
        session
            .services
            .agent_control
            .resume_agent_from_rollout(
                config,
                receiver_thread_id,
                thread_spawn_source(
                    session.conversation_id,
                    &turn.session_source,
                    child_depth,
                    /*agent_role*/ None,
                    /*task_name*/ None,
                )?,
            )
            .await
            .map_err(|err| collab_agent_error(receiver_thread_id, err))?;
    }
    session
        .services
        .agent_control
        .send_input(
            receiver_thread_id,
            vec![UserInput::Text {
                text: prompt.to_string(),
                text_elements: Vec::new(),
            }]
            .into(),
        )
        .await
        .map_err(|err| collab_agent_error(receiver_thread_id, err))?;
    Ok(receiver_thread_id)
}

async fn wait_for_kimi_agent_completion(
    session: &Session,
    agent_id: ThreadId,
) -> Result<ProtocolAgentStatus, FunctionCallError> {
    let mut status_rx = session
        .services
        .agent_control
        .subscribe_status(agent_id)
        .await
        .map_err(|err| FunctionCallError::RespondToModel(format!("Agent failed: {err}")))?;
    let mut status = status_rx.borrow().clone();
    while matches!(
        status,
        AgentStatus::PendingInit | AgentStatus::Running | AgentStatus::Interrupted
    ) {
        if status_rx.changed().await.is_err() {
            return Ok(session.services.agent_control.get_status(agent_id).await);
        }
        status = status_rx.borrow().clone();
    }
    Ok(status)
}

fn build_kimi_agent_prompt(prompt: &str, prompt_prefix: Option<&'static str>) -> String {
    match prompt_prefix {
        Some(prefix) => format!("{prefix}\n\n{prompt}"),
        None => prompt.to_string(),
    }
}

fn kimi_agent_profile(subagent_type: Option<&str>) -> (Option<&'static str>, Option<&'static str>) {
    match subagent_type.map(str::trim) {
        Some("explore") => (Some("explorer"), None),
        Some("plan") => (
            None,
            Some(
                "Approach this as a planning specialist. Stay read-only, focus on implementation strategy, identify the key files, and explain trade-offs before recommending a path.",
            ),
        ),
        Some("coder") | Some("") | None => (None, None),
        Some(_) => (None, None),
    }
}
