use codex_protocol::ThreadId;
#[cfg(test)]
use codex_protocol::config_types::EnvironmentVariablePattern;
use codex_protocol::config_types::ShellEnvironmentPolicy;
use codex_protocol::models::ActivePermissionProfile;
use codex_protocol::shell_environment;
use std::collections::HashMap;

pub use codex_protocol::shell_environment::CODEX_THREAD_ID_ENV_VAR;

const AI_AGENT_ENV_VAR: &str = "AI_AGENT";
const OPEN_INTERPRETER_AI_AGENT: &str = "open-interpreter";

/// Informational name of the active permission profile. Child processes can
/// overwrite this value, so it must not be treated as proof of enforcement.
pub const CODEX_PERMISSION_PROFILE_ENV_VAR: &str = "CODEX_PERMISSION_PROFILE";

/// Construct an environment map based on the rules in the specified policy. The
/// resulting map can be passed directly to `Command::envs()` after calling
/// `env_clear()` to ensure no unintended variables are leaked to the spawned
/// process.
///
/// The derivation follows the algorithm documented in the struct-level comment
/// for [`ShellEnvironmentPolicy`].
///
/// `CODEX_THREAD_ID` is injected when a thread id is provided, even when
/// `include_only` is set. `AI_AGENT` is always present so child processes can
/// detect Open Interpreter through a vendor-neutral marker.
pub fn create_env(
    policy: &ShellEnvironmentPolicy,
    thread_id: Option<ThreadId>,
) -> HashMap<String, String> {
    let thread_id = thread_id.map(|thread_id| thread_id.to_string());
    let mut env = shell_environment::create_env(policy, thread_id.as_deref());
    inject_ai_agent_env(&mut env);
    env
}

fn inject_ai_agent_env(env: &mut HashMap<String, String>) {
    let existing = env.iter().find(|(key, _)| {
        if cfg!(windows) {
            key.eq_ignore_ascii_case(AI_AGENT_ENV_VAR)
        } else {
            key.as_str() == AI_AGENT_ENV_VAR
        }
    });
    if existing.is_some_and(|(_, value)| !value.trim().is_empty()) {
        return;
    }
    if cfg!(windows) {
        env.retain(|key, _| !key.eq_ignore_ascii_case(AI_AGENT_ENV_VAR));
    }
    env.insert(
        AI_AGENT_ENV_VAR.to_string(),
        OPEN_INTERPRETER_AI_AGENT.to_string(),
    );
}

/// Injects the selected named permission profile into a shell tool's environment.
///
/// This is applied after the shell environment policy so the runtime-selected
/// profile wins over inherited or configured values.
pub(crate) fn inject_permission_profile_env(
    env: &mut HashMap<String, String>,
    active_permission_profile: Option<&ActivePermissionProfile>,
) {
    if cfg!(windows) {
        env.retain(|key, _| !key.eq_ignore_ascii_case(CODEX_PERMISSION_PROFILE_ENV_VAR));
    } else {
        env.remove(CODEX_PERMISSION_PROFILE_ENV_VAR);
    }
    if let Some(active_permission_profile) = active_permission_profile {
        env.insert(
            CODEX_PERMISSION_PROFILE_ENV_VAR.to_string(),
            active_permission_profile.id.clone(),
        );
    }
}

#[cfg(all(test, target_os = "windows"))]
fn create_env_from_vars<I>(
    vars: I,
    policy: &ShellEnvironmentPolicy,
    thread_id: Option<ThreadId>,
) -> HashMap<String, String>
where
    I: IntoIterator<Item = (String, String)>,
{
    let thread_id = thread_id.map(|thread_id| thread_id.to_string());
    shell_environment::create_env_from_vars(vars, policy, thread_id.as_deref())
}

#[cfg(test)]
fn populate_env<I>(
    vars: I,
    policy: &ShellEnvironmentPolicy,
    thread_id: Option<ThreadId>,
) -> HashMap<String, String>
where
    I: IntoIterator<Item = (String, String)>,
{
    let thread_id = thread_id.map(|thread_id| thread_id.to_string());
    shell_environment::populate_env(vars, policy, thread_id.as_deref())
}

#[cfg(test)]
#[path = "exec_env_tests.rs"]
mod tests;
