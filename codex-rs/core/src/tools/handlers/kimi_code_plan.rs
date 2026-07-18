use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::LazyLock;
use std::sync::Mutex;

use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolOutput;
use crate::tools::context::boxed_tool_output;
use crate::tools::handlers::harness_fs;

static ACTIVE_PLANS: LazyLock<Mutex<HashMap<String, PathBuf>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub(crate) async fn handle_enter(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let session_id = invocation.session.session_id().to_string();
    if ACTIVE_PLANS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .contains_key(&session_id)
    {
        return Ok(output(
            "Plan mode is already active. Use ExitPlanMode when the plan is ready.".to_string(),
            /*success*/ false,
        ));
    }
    let path = invocation
        .turn
        .config
        .codex_home
        .as_path()
        .join("sessions")
        .join(&session_id)
        .join("agents")
        .join("main")
        .join("plans")
        .join(format!("{}.md", uuid::Uuid::new_v4()));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| {
            FunctionCallError::RespondToModel(format!("Failed to enter plan mode: {err}"))
        })?;
    }
    ACTIVE_PLANS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(session_id, path.clone());
    Ok(output(entered_message(&path), /*success*/ true))
}

pub(crate) async fn handle_exit(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let session_id = invocation.session.session_id().to_string();
    let Some(path) = ACTIVE_PLANS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&session_id)
        .cloned()
    else {
        return Ok(output(
            "ExitPlanMode can only be called while plan mode is active. Use EnterPlanMode (or /plan) first.".to_string(),
            /*success*/ false,
        ));
    };
    let plan = match std::fs::read_to_string(&path) {
        Ok(plan) if !plan.trim().is_empty() => plan,
        Ok(_) | Err(_) => {
            return Ok(output(
                format!(
                    "No plan file found. Write your plan to {} first, then call ExitPlanMode.",
                    path.display()
                ),
                /*success*/ false,
            ));
        }
    };
    ACTIVE_PLANS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(&session_id);
    Ok(output(
        format!(
            "Exited plan mode. Plan mode deactivated. All tools are now available.\nNote: this plan was auto-approved without user review — the user has NOT explicitly approved it. Follow the user's original instructions on whether to proceed with execution; if they asked you to stop, wait, or only summarize after planning, do not start executing.\nPlan saved to: {}\n\n## Plan (auto-approved, not user-reviewed):\n{plan}",
            path.display()
        ),
        /*success*/ true,
    ))
}

pub(crate) fn is_current_plan_path(invocation: &ToolInvocation, model_path: &str) -> bool {
    let Ok(path) = harness_fs::resolve_model_path(invocation, model_path) else {
        return false;
    };
    ACTIVE_PLANS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&invocation.session.session_id().to_string())
        .is_some_and(|active| active == &path)
}

fn entered_message(path: &std::path::Path) -> String {
    format!(
        "Plan mode is now active. Your workflow:\n\nPlan file: {}\n\n1. Use read-only tools (Read, Grep, Glob) to investigate the codebase. Use Bash only when needed.\n2. Design a concrete, step-by-step plan.\n3. Write the plan to the plan file with Write or Edit.\n4. When the plan is ready, call ExitPlanMode for user approval.\n\nDo NOT edit files other than the plan file while plan mode is active.\nUse Bash only when needed; Bash follows the normal permission mode and rules.",
        path.display()
    )
}

fn output(text: String, success: bool) -> Box<dyn ToolOutput> {
    boxed_tool_output(FunctionToolOutput::from_text(text, Some(success)))
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn enter_message_names_the_plan_file_and_workflow() {
        let message = entered_message(std::path::Path::new("/tmp/plan.md"));
        assert_eq!(
            message,
            "Plan mode is now active. Your workflow:\n\nPlan file: /tmp/plan.md\n\n1. Use read-only tools (Read, Grep, Glob) to investigate the codebase. Use Bash only when needed.\n2. Design a concrete, step-by-step plan.\n3. Write the plan to the plan file with Write or Edit.\n4. When the plan is ready, call ExitPlanMode for user approval.\n\nDo NOT edit files other than the plan file while plan mode is active.\nUse Bash only when needed; Bash follows the normal permission mode and rules."
        );
    }
}
