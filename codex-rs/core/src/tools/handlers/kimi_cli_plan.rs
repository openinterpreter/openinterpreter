use crate::function_tool::FunctionCallError;
use crate::session::session::SessionSettingsUpdate;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::parse_kimi_arguments;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;
use codex_protocol::config_types::ModeKind;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::request_user_input::RequestUserInputArgs;
use codex_protocol::request_user_input::RequestUserInputQuestion;
use codex_protocol::request_user_input::RequestUserInputQuestionOption;
use codex_tools::request_user_input_unavailable_message;
use serde::Deserialize;

pub struct KimiEnterPlanModeHandler;
pub struct KimiExitPlanModeHandler;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KimiExitPlanModeArgs {
    options: Option<Vec<KimiExitPlanOption>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KimiExitPlanOption {
    label: String,
    #[allow(dead_code)]
    description: Option<String>,
}

impl ToolHandler for KimiEnterPlanModeHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolInvocation {
            session,
            turn,
            call_id,
            payload,
            ..
        } = invocation;
        let ToolPayload::Function { .. } = payload else {
            return Err(FunctionCallError::RespondToModel(
                "EnterPlanMode received unsupported payload".to_string(),
            ));
        };
        let current_mode = session.collaboration_mode().await;
        if current_mode.mode == ModeKind::Plan {
            return Ok(system_message_output("Already in plan mode."));
        }
        if !plan_mode_approved(
            session.as_ref(),
            turn.as_ref(),
            call_id,
            PlanApprovalKind::Enter,
        )
        .await?
        {
            return Err(FunctionCallError::RespondToModel(
                "User declined entering plan mode.".to_string(),
            ));
        }

        let mut next_mode = current_mode;
        next_mode.mode = ModeKind::Plan;
        session
            .update_settings(SessionSettingsUpdate {
                collaboration_mode: Some(next_mode),
                ..Default::default()
            })
            .await
            .map_err(|err| {
                FunctionCallError::RespondToModel(format!("failed to enter plan mode: {err}"))
            })?;
        Ok(system_message_output("Entered plan mode."))
    }
}

impl ToolHandler for KimiExitPlanModeHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolInvocation {
            session,
            turn,
            call_id,
            payload,
            ..
        } = invocation;
        let ToolPayload::Function { arguments } = payload else {
            return Err(FunctionCallError::RespondToModel(
                "ExitPlanMode received unsupported payload".to_string(),
            ));
        };
        let args: KimiExitPlanModeArgs = parse_kimi_arguments(&arguments)?;
        let current_mode = session.collaboration_mode().await;
        if current_mode.mode != ModeKind::Plan {
            return Ok(system_message_output("Plan mode is not active."));
        }
        if !plan_mode_approved(
            session.as_ref(),
            turn.as_ref(),
            call_id,
            PlanApprovalKind::Exit(args.options),
        )
        .await?
        {
            return Err(FunctionCallError::RespondToModel(
                "User declined the plan review.".to_string(),
            ));
        }

        let mut next_mode = current_mode;
        next_mode.mode = ModeKind::Default;
        session
            .update_settings(SessionSettingsUpdate {
                collaboration_mode: Some(next_mode),
                ..Default::default()
            })
            .await
            .map_err(|err| {
                FunctionCallError::RespondToModel(format!("failed to exit plan mode: {err}"))
            })?;
        Ok(system_message_output("Exited plan mode."))
    }
}

enum PlanApprovalKind {
    Enter,
    Exit(Option<Vec<KimiExitPlanOption>>),
}

async fn plan_mode_approved(
    session: &crate::session::Session,
    turn: &crate::session::TurnContext,
    call_id: String,
    approval_kind: PlanApprovalKind,
) -> Result<bool, FunctionCallError> {
    if kimi_plan_approval_is_preapproved(turn) {
        return Ok(true);
    }

    let mode = session.collaboration_mode().await.mode;
    if let Some(message) = request_user_input_unavailable_message(
        mode,
        turn.tools_config.default_mode_request_user_input,
    ) {
        return Err(FunctionCallError::RespondToModel(message));
    }

    let (question, options) = match approval_kind {
        PlanApprovalKind::Enter => (
            "Enter plan mode before implementation?",
            vec![
                RequestUserInputQuestionOption {
                    label: "Enter plan mode (Recommended)".to_string(),
                    description: "Pause implementation and work in planning mode first."
                        .to_string(),
                },
                RequestUserInputQuestionOption {
                    label: "Keep current mode".to_string(),
                    description: "Continue without switching into plan mode.".to_string(),
                },
            ],
        ),
        PlanApprovalKind::Exit(extra_options) => {
            let mut options = vec![
                RequestUserInputQuestionOption {
                    label: "Approve plan (Recommended)".to_string(),
                    description: "Leave plan mode and proceed with this plan.".to_string(),
                },
                RequestUserInputQuestionOption {
                    label: "Keep planning".to_string(),
                    description: "Stay in plan mode and refine the approach.".to_string(),
                },
            ];
            if let Some(extra_options) = extra_options {
                options.extend(extra_options.into_iter().map(|option| {
                    RequestUserInputQuestionOption {
                        label: option.label,
                        description: option.description.unwrap_or_default(),
                    }
                }));
            }
            ("Approve this plan and exit plan mode?", options)
        }
    };

    let response = session
        .request_user_input(
            turn,
            call_id,
            RequestUserInputArgs {
                questions: vec![RequestUserInputQuestion {
                    id: "kimi-plan-approval".to_string(),
                    header: "Plan mode".to_string(),
                    question: question.to_string(),
                    is_other: false,
                    is_secret: false,
                    options: Some(options),
                }],
            },
        )
        .await
        .ok_or_else(|| {
            FunctionCallError::RespondToModel(
                "plan mode approval was cancelled before receiving a response".to_string(),
            )
        })?;

    Ok(response
        .answers
        .get("kimi-plan-approval")
        .and_then(|answer| answer.answers.first())
        .is_some_and(|answer| {
            answer == "Enter plan mode (Recommended)" || answer == "Approve plan (Recommended)"
        }))
}

fn kimi_plan_approval_is_preapproved(turn: &crate::session::TurnContext) -> bool {
    std::env::var_os("OPEN_INTERPRETER_KIMI_CLI_YOLO").is_some()
        || turn.approval_policy.value() == AskForApproval::Never
}

fn system_message_output(text: &str) -> FunctionToolOutput {
    FunctionToolOutput::from_content(
        vec![
            codex_protocol::models::FunctionCallOutputContentItem::InputText {
                text: format!("<system>{text}</system>"),
            },
        ],
        Some(true),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::tests::make_session_and_context;
    use crate::tools::context::ToolCallSource;
    use crate::tools::context::ToolInvocation;
    use crate::tools::context::ToolPayload;
    use crate::tools::registry::ToolHandler;
    use crate::turn_diff_tracker::TurnDiffTracker;
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    fn invocation(
        session: Arc<crate::session::session::Session>,
        turn: Arc<crate::session::TurnContext>,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> ToolInvocation {
        ToolInvocation {
            session,
            turn,
            cancellation_token: tokio_util::sync::CancellationToken::new(),
            tracker: Arc::new(Mutex::new(TurnDiffTracker::default())),
            call_id: "call_1".to_string(),
            tool_name: codex_tools::ToolName::plain(tool_name),
            source: ToolCallSource::Direct,
            payload: ToolPayload::Function {
                arguments: arguments.to_string(),
            },
        }
    }

    #[tokio::test]
    async fn enter_plan_mode_auto_accepts_when_approval_policy_never() {
        let (session, mut turn) = make_session_and_context().await;
        turn.approval_policy
            .set(AskForApproval::Never)
            .expect("test setup can set approval policy");
        let session = Arc::new(session);
        let turn = Arc::new(turn);

        let output = KimiEnterPlanModeHandler
            .handle(invocation(
                Arc::clone(&session),
                Arc::clone(&turn),
                "EnterPlanMode",
                json!({}),
            ))
            .await
            .expect("enter plan mode succeeds")
            .into_text();

        assert_eq!(output, "<system>Entered plan mode.</system>");
        assert_eq!(session.collaboration_mode().await.mode, ModeKind::Plan);
    }

    #[tokio::test]
    async fn exit_plan_mode_auto_accepts_when_approval_policy_never() {
        let (session, mut turn) = make_session_and_context().await;
        turn.approval_policy
            .set(AskForApproval::Never)
            .expect("test setup can set approval policy");
        let session = Arc::new(session);
        let turn = Arc::new(turn);

        let mut plan_mode = session.collaboration_mode().await;
        plan_mode.mode = ModeKind::Plan;
        session
            .update_settings(SessionSettingsUpdate {
                collaboration_mode: Some(plan_mode),
                ..Default::default()
            })
            .await
            .expect("test setup can enter plan mode");

        let output = KimiExitPlanModeHandler
            .handle(invocation(
                Arc::clone(&session),
                Arc::clone(&turn),
                "ExitPlanMode",
                json!({
                    "options": [
                        {
                            "label": "Revise plan",
                            "description": "Stay in planning and revise."
                        }
                    ]
                }),
            ))
            .await
            .expect("exit plan mode succeeds")
            .into_text();

        assert_eq!(output, "<system>Exited plan mode.</system>");
        assert_eq!(session.collaboration_mode().await.mode, ModeKind::Default);
    }
}
