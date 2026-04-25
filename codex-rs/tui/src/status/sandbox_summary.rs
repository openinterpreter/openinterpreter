use codex_protocol::protocol::NetworkAccess;
use codex_protocol::protocol::SandboxPolicy;

pub(crate) fn summarize_sandbox_policy(sandbox_policy: &SandboxPolicy) -> String {
    match sandbox_policy {
        SandboxPolicy::DangerFullAccess => "danger-full-access".to_string(),
        SandboxPolicy::ReadOnly { network_access, .. } => {
            let mut summary = "read-only".to_string();
            if *network_access {
                summary.push_str(" (network access enabled)");
            }
            summary
        }
        SandboxPolicy::ExternalSandbox { network_access } => {
            let mut summary = "external-sandbox".to_string();
            if matches!(network_access, NetworkAccess::Enabled) {
                summary.push_str(" (network access enabled)");
            }
            summary
        }
        SandboxPolicy::WorkspaceWrite {
            writable_roots,
            network_access,
            exclude_tmpdir_env_var,
            exclude_slash_tmp,
            read_only_access: _,
        } => {
            let mut summary = "workspace-write".to_string();

            let mut writable_entries = vec!["workdir".to_string()];
            if !*exclude_slash_tmp {
                writable_entries.push("/tmp".to_string());
            }
            if !*exclude_tmpdir_env_var {
                writable_entries.push("$TMPDIR".to_string());
            }
            writable_entries.extend(
                writable_roots
                    .iter()
                    .map(|p| p.to_string_lossy().to_string()),
            );

            summary.push_str(&format!(" [{}]", writable_entries.join(", ")));
            if *network_access {
                summary.push_str(" (network access enabled)");
            }
            summary
        }
    }
}
