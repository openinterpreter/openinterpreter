use crate::exec::ExecCapturePolicy;
use crate::exec::ExecParams;
use crate::exec_env::create_env;
use crate::exec_policy::ExecApprovalRequest;
use crate::function_tool::FunctionCallError;
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
use codex_protocol::exec_output::ExecToolCallOutput;
use codex_protocol::models::SandboxPermissions;
use codex_protocol::protocol::ExecCommandSource;
use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_output_truncation::TruncationPolicy;
use serde::Deserialize;
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::LazyLock;
use std::sync::Mutex;

pub struct MinimalBashHandler;

const MINIMAL_BASH_EMPTY_OUTPUT: &str = "<system>Command executed successfully.</system>";
const MINIMAL_BASH_MAX_TIMEOUT_MS: u64 = 300_000;
const CWD_SENTINEL: &str = "__OI_MINIMAL_CWD__";

static PERSISTENT_CWDS: LazyLock<Mutex<HashMap<String, PathBuf>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Deserialize)]
struct MinimalBashArgs {
    command: String,
    timeout: Option<u64>,
}

impl ToolHandler for MinimalBashHandler {
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
                "bash received unsupported payload".to_string(),
            ));
        };
        let args: MinimalBashArgs = parse_arguments(&arguments)?;
        let Some(_environment) = turn.environment.as_ref() else {
            return Err(FunctionCallError::RespondToModel(
                "bash is unavailable in this session".to_string(),
            ));
        };

        let conversation_key = session.conversation_id.to_string();
        let cwd = persistent_cwd(&conversation_key)
            .and_then(|path| AbsolutePathBuf::from_absolute_path(path).ok())
            .unwrap_or_else(|| turn.cwd.clone());
        let timeout_ms = Some(
            args.timeout
                .unwrap_or(120)
                .saturating_mul(1000)
                .min(MINIMAL_BASH_MAX_TIMEOUT_MS),
        );
        let wrapped_command = wrap_persistent_command(&cwd, &args.command);
        let command = session
            .user_shell()
            .derive_exec_args(&wrapped_command, turn.tools_config.allow_login_shell);
        let exec_params = ExecParams {
            command: command.clone(),
            cwd: cwd.clone(),
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
            justification: None,
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
        let effective_permissions = apply_granted_turn_permissions(
            session.as_ref(),
            turn.cwd.as_path(),
            SandboxPermissions::UseDefault,
            None,
        )
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
            explicit_env_overrides: turn.shell_environment_policy.r#set.clone(),
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
                let output = strip_cwd_sentinel_and_update(output, &conversation_key);
                emitter
                    .emit(event_ctx, ToolEventStage::Success(output.clone()))
                    .await;
                let text = minimal_bash_output_text(&output, turn.truncation_policy);
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
                let output = strip_cwd_sentinel_and_update(*output, &conversation_key);
                emitter
                    .emit(
                        event_ctx,
                        ToolEventStage::Failure(ToolEventFailure::Output(output.clone())),
                    )
                    .await;
                let text = minimal_bash_output_text(&output, turn.truncation_policy);
                Err(FunctionCallError::RespondToModel(text))
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

fn persistent_cwd(conversation_key: &str) -> Option<PathBuf> {
    PERSISTENT_CWDS
        .lock()
        .ok()
        .and_then(|cwds| cwds.get(conversation_key).cloned())
}

fn set_persistent_cwd(conversation_key: &str, cwd: PathBuf) {
    if let Ok(mut cwds) = PERSISTENT_CWDS.lock() {
        cwds.insert(conversation_key.to_string(), cwd);
    }
}

fn wrap_persistent_command(cwd: &Path, command: &str) -> String {
    format!(
        "cd {}\n{}\n__oi_minimal_status=$?\nprintf '\\n{}%s\\n' \"$PWD\"\nexit $__oi_minimal_status",
        shell_quote(cwd),
        command,
        CWD_SENTINEL
    )
}

fn shell_quote(path: &Path) -> String {
    let value = path.to_string_lossy();
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn strip_cwd_sentinel_and_update(
    mut output: ExecToolCallOutput,
    conversation_key: &str,
) -> ExecToolCallOutput {
    let Some((cleaned, cwd)) = split_cwd_sentinel(&output.aggregated_output.text) else {
        return output;
    };
    output.aggregated_output.text = cleaned;
    output.stdout.text = strip_cwd_sentinel_from_stream(&output.stdout.text);
    output.stderr.text = strip_cwd_sentinel_from_stream(&output.stderr.text);
    set_persistent_cwd(conversation_key, PathBuf::from(cwd));
    output
}

fn split_cwd_sentinel(text: &str) -> Option<(String, String)> {
    let marker_index = text.rfind(CWD_SENTINEL)?;
    let before_marker = text[..marker_index]
        .trim_end_matches(['\r', '\n'])
        .to_string();
    let cwd_start = marker_index + CWD_SENTINEL.len();
    let cwd = text[cwd_start..].lines().next()?.trim().to_string();
    if cwd.is_empty() {
        None
    } else {
        Some((before_marker, cwd))
    }
}

fn strip_cwd_sentinel_from_stream(text: &str) -> String {
    split_cwd_sentinel(text)
        .map(|(cleaned, _)| cleaned)
        .unwrap_or_else(|| text.to_string())
}

fn minimal_bash_output_text(
    output: &ExecToolCallOutput,
    truncation_policy: TruncationPolicy,
) -> String {
    let text = crate::tools::format_exec_output_str(output, truncation_policy);
    if text.is_empty() {
        MINIMAL_BASH_EMPTY_OUTPUT.to_string()
    } else {
        text
    }
}
