//! Resolves an executor sandbox context to a concrete local sandbox implementation.

use codex_file_system::FileSystemSandboxContext;
use codex_file_system::WindowsSandboxSelection;
use codex_protocol::config_types::WindowsSandboxLevel;
use codex_protocol::models::PermissionProfile;
use codex_sandboxing::SandboxManager;
use codex_sandboxing::SandboxSelection;
use codex_sandboxing::SandboxType;
use codex_sandboxing::SandboxablePreference;

pub(crate) fn validate_remote_mxc(info: &crate::EnvironmentInfo) -> std::io::Result<()> {
    if !info.capabilities.windows_mxc {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "exec-server does not support the requested MXC Windows sandbox; update the executor",
        ));
    }
    Ok(())
}

pub(crate) fn sandbox_selection(
    context: &FileSystemSandboxContext,
) -> (SandboxSelection, WindowsSandboxLevel) {
    let level = match context.windows_sandbox_selection {
        WindowsSandboxSelection::Disabled => WindowsSandboxLevel::Disabled,
        WindowsSandboxSelection::RestrictedToken => WindowsSandboxLevel::RestrictedToken,
        WindowsSandboxSelection::Elevated => WindowsSandboxLevel::Elevated,
        WindowsSandboxSelection::Mxc => {
            return (
                SandboxSelection::Explicit(SandboxType::WindowsMxc),
                WindowsSandboxLevel::Disabled,
            );
        }
    };
    (SandboxSelection::RequirePlatform, level)
}

pub(crate) fn select_sandbox(
    manager: &SandboxManager,
    permission_profile: &PermissionProfile,
    sandbox_context: &FileSystemSandboxContext,
    has_managed_network_requirements: bool,
) -> (SandboxType, Option<WindowsSandboxLevel>) {
    let windows_sandbox_level = match sandbox_context.windows_sandbox_selection {
        WindowsSandboxSelection::Disabled => WindowsSandboxLevel::Disabled,
        WindowsSandboxSelection::RestrictedToken => WindowsSandboxLevel::RestrictedToken,
        WindowsSandboxSelection::Elevated => WindowsSandboxLevel::Elevated,
        WindowsSandboxSelection::Mxc => return (SandboxType::WindowsMxc, None),
    };
    let windows_sandbox_type = match windows_sandbox_level {
        WindowsSandboxLevel::Disabled => SandboxType::None,
        WindowsSandboxLevel::RestrictedToken | WindowsSandboxLevel::Elevated => {
            SandboxType::WindowsRestrictedToken
        }
    };
    let sandbox_type = manager.select_initial(
        permission_profile,
        SandboxablePreference::Require,
        windows_sandbox_type,
        has_managed_network_requirements,
    );
    (sandbox_type, Some(windows_sandbox_level))
}
