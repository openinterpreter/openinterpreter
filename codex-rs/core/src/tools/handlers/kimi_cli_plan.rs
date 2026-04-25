use crate::function_tool::FunctionCallError;
use crate::session::session::SessionSettingsUpdate;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;
use codex_protocol::config_types::ModeKind;
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
        let args: KimiExitPlanModeArgs = parse_arguments(&arguments)?;
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
    if std::env::var_os("OPEN_INTERPRETER_KIMI_CLI_YOLO").is_some() {
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
