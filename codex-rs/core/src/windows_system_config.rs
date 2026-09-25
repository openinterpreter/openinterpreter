//! Telemetry for the default Windows system-config namespace.

use codex_config::loader::WindowsSystemConfigNamespaceProbe as Probe;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

const NAMESPACE_SQUATTING_PROBE_METRIC: &str =
    "codex.windows_system_config.namespace_squatting_probe";
static NAMESPACE_SQUATTING_PROBE_RECORDED: AtomicBool = AtomicBool::new(false);

/// Records the coarse system-config namespace probe once after metrics exist.
pub(crate) fn emit_namespace_squatting_probe() {
    let Some(metrics) = codex_otel::global() else {
        return;
    };
    if NAMESPACE_SQUATTING_PROBE_RECORDED
        .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
        .is_err()
    {
        return;
    }
    let result = match codex_config::loader::probe_windows_system_config_namespace() {
        Probe::Missing => "missing",
        Probe::Expected => "expected",
        Probe::UnexpectedType => "unexpected_type",
        Probe::UnexpectedOwner => "unexpected_owner",
        Probe::StandardUserMutationAcl => "standard_user_acl",
        Probe::CheckError => "check_error",
    };
    let _ = metrics.counter(
        NAMESPACE_SQUATTING_PROBE_METRIC,
        /*inc*/ 1,
        &[("result", result)],
    );
}
