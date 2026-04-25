use crate::exec::ExecCapturePolicy;
use crate::exec::ExecParams;
use crate::exec_env::create_env;
use crate::exec_policy::ExecApprovalRequest;
use crate::function_tool::FunctionCallError;
use crate::session::TurnContext;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::events::ToolEmitter;
use crate::tools::events::ToolEventCtx;
use crate::tools::events::ToolEventFailure;
use crate::tools::events::ToolEventStage;
use crate::tools::handlers::apply_granted_turn_permissions;
use crate::tools::handlers::parse_arguments;
use crate::tools::orchestrator::ToolOrchestrator;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;
use crate::tools::runtimes::shell::ShellRequest;
use crate::tools::runtimes::shell::ShellRuntime;
use crate::tools::runtimes::shell::ShellRuntimeBackend;
use crate::tools::sandboxing::ToolError;
use codex_protocol::error::CodexErr;
use codex_protocol::error::SandboxErr;
use codex_protocol::models::SandboxPermissions;
use codex_protocol::protocol::ExecCommandSource;
use serde::Deserialize;
use serde_json::Value as JsonValue;
use std::collections::HashMap;

pub struct KimiShellHandler;

const KIMI_SHELL_EMPTY_OUTPUT: &str = "<system>Command executed successfully.</system>";
const KIMI_SHELL_MAX_TIMEOUT_MS: u64 = 300_000;

#[derive(Deserialize)]
struct KimiShellArgs {
    command: String,
    timeout: Option<u64>,
    run_in_background: Option<bool>,
    description: Option<String>,
}

impl ToolHandler for KimiShellHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn is_mutating(&self, _invocation: &ToolInvocation) -> bool {
        true
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolInvocation {
            session,
            turn,
            call_id,
            tool_name,
            payload,
            ..
        } = invocation;
        let ToolPayload::Function { arguments } = payload else {
            return Err(FunctionCallError::RespondToModel(
                "Shell received unsupported payload".to_string(),
            ));
        };
        let args: KimiShellArgs = parse_arguments(&arguments)?;
        if args.run_in_background.unwrap_or(false) {
            return Err(FunctionCallError::RespondToModel(
                "Shell run_in_background is not implemented for the kimi-cli harness yet."
                    .to_string(),
            ));
        }
        let Some(_environment) = turn.environment.as_ref() else {
            return Err(FunctionCallError::RespondToModel(
                "Shell is unavailable in this session".to_string(),
            ));
        };
        let timeout_ms = Some(
            args.timeout
                .unwrap_or(60)
                .saturating_mul(1000)
                .min(KIMI_SHELL_MAX_TIMEOUT_MS),
        );
        let command = session
            .user_shell()
            .derive_exec_args(&args.command, turn.tools_config.allow_login_shell);
        let exec_params = ExecParams {
            command: command.clone(),
            cwd: turn.cwd.clone(),
            expiration: timeout_ms.into(),
            capture_policy: ExecCapturePolicy::ShellTool,
            env: create_env(
                &turn.shell_environment_policy,
                Some(session.conversation_id),
            ),
            network: turn.network.clone(),
            sandbox_permissions: SandboxPermissions::UseDefault,
            windows_sandbox_level: turn.windows_sandbox_level,
            windows_sandbox_private_desktop: turn
                .config
                .permissions
                .windows_sandbox_private_desktop,
            justification: args.description.clone(),
            arg0: None,
        };
        let emitter = ToolEmitter::shell(
            exec_params.command.clone(),
            exec_params.cwd.clone(),
            ExecCommandSource::Agent,
            /*freeform*/ false,
        );
        let event_ctx = ToolEventCtx::new(
            session.as_ref(),
            turn.as_ref(),
            &call_id,
            /*turn_diff_tracker*/ None,
        );
        emitter.begin(event_ctx).await;
        let effective_permissions =
            apply_granted_turn_permissions(session.as_ref(), SandboxPermissions::UseDefault, None)
                .await;
        let exec_approval_requirement = session
            .services
            .exec_policy
            .create_exec_approval_requirement_for_command(ExecApprovalRequest {
                command: &exec_params.command,
                approval_policy: turn.approval_policy.value(),
                sandbox_policy: turn.sandbox_policy.get(),
                file_system_sandbox_policy: &turn.file_system_sandbox_policy,
                sandbox_permissions: if effective_permissions.permissions_preapproved {
                    SandboxPermissions::UseDefault
                } else {
                    effective_permissions.sandbox_permissions
                },
                prefix_rule: None,
            })
            .await;
        let request = ShellRequest {
            command: exec_params.command.clone(),
            hook_command: codex_shell_command::parse_command::shlex_join(&exec_params.command),
            cwd: exec_params.cwd.clone(),
            timeout_ms,
            env: exec_params.env.clone(),
            explicit_env_overrides: HashMap::new(),
            network: exec_params.network.clone(),
            sandbox_permissions: effective_permissions.sandbox_permissions,
            additional_permissions: None,
            #[cfg(unix)]
            additional_permissions_preapproved: effective_permissions.permissions_preapproved,
            justification: exec_params.justification.clone(),
            exec_approval_requirement,
        };

        let mut orchestrator = ToolOrchestrator::new();
        let mut runtime = ShellRuntime::for_shell_command(ShellRuntimeBackend::ShellCommandClassic);
        let tool_ctx = crate::tools::sandboxing::ToolCtx {
            session: session.clone(),
            turn: turn.clone(),
            call_id: call_id.clone(),
            tool_name: tool_name.display(),
        };
        let result = orchestrator
            .run(
                &mut runtime,
                &request,
                &tool_ctx,
                &turn,
                turn.approval_policy.value(),
            )
            .await
            .map(|output| output.output);

        match result {
            Ok(output) => {
                emitter
                    .emit(event_ctx, ToolEventStage::Success(output.clone()))
                    .await;
                let text = kimi_shell_output_text(&output, turn.as_ref());
                if output.exit_code == 0 {
                    Ok(FunctionToolOutput {
                        body: vec![
                            codex_protocol::models::FunctionCallOutputContentItem::InputText {
                                text: text.clone(),
                            },
                        ],
                        success: Some(true),
                        post_tool_use_response: Some(JsonValue::String(text)),
                    })
                } else {
                    Err(FunctionCallError::RespondToModel(text))
                }
            }
            Err(ToolError::Codex(CodexErr::Sandbox(SandboxErr::Timeout { output })))
            | Err(ToolError::Codex(CodexErr::Sandbox(SandboxErr::Denied { output, .. }))) => {
                let output = *output;
                emitter
                    .emit(
                        event_ctx,
                        ToolEventStage::Failure(ToolEventFailure::Output(output.clone())),
                    )
                    .await;
                let text = kimi_shell_output_text(&output, turn.as_ref());
                let fallback = if text.is_empty() {
                    KIMI_SHELL_EMPTY_OUTPUT.to_string()
                } else {
                    text
                };
                Err(FunctionCallError::RespondToModel(fallback))
            }
            Err(ToolError::Rejected(message)) => {
                emitter
                    .emit(
                        event_ctx,
                        ToolEventStage::Failure(ToolEventFailure::Rejected(message.clone())),
                    )
                    .await;
                Err(FunctionCallError::RespondToModel(message))
            }
            Err(ToolError::Codex(err)) => {
                let message = format!("execution error: {err:?}");
                emitter
                    .emit(
                        event_ctx,
                        ToolEventStage::Failure(ToolEventFailure::Message(message.clone())),
                    )
                    .await;
                Err(FunctionCallError::RespondToModel(message))
            }
        }
    }
}

fn kimi_shell_output_text(
    output: &codex_protocol::exec_output::ExecToolCallOutput,
    turn: &TurnContext,
) -> String {
    let text = crate::tools::format_exec_output_str(output, turn.truncation_policy);
    if text.is_empty() {
        KIMI_SHELL_EMPTY_OUTPUT.to_string()
    } else {
        text
    }
}
