use std::ffi::OsStr;
use std::io;
use std::path::PathBuf;

use crate::startup_trace::record_startup_trace_event;

pub const INTERPRETER_HOME_ENV_VAR: &str = "INTERPRETER_HOME";
pub const OPEN_INTERPRETER_HOME_ENV_VAR: &str = "OPEN_INTERPRETER_HOME";
pub const INTERPRETER_DISABLE_SYSTEM_IMPORT_ENV_VAR: &str = "INTERPRETER_DISABLE_SYSTEM_IMPORT";
pub const INTERPRETER_FORCE_PROVIDER_ONBOARDING_ENV_VAR: &str =
    "INTERPRETER_FORCE_PROVIDER_ONBOARDING";
pub const FRESH_HOME_PROVIDER_ONBOARDING_MARKER_FILE: &str = ".fresh_home_provider_onboarding";
const CODEX_HOME_ENV_VAR: &str = "CODEX_HOME";
const OPEN_INTERPRETER_BRAND_ENV_VAR: &str = "OPEN_INTERPRETER_BRAND";
const DEFAULT_OPEN_INTERPRETER_HOME_DIR: &str = ".openinterpreter";
const CONFIG_TOML_FILE: &str = "config.toml";
const AUTH_JSON_FILE: &str = "auth.json";

pub fn ensure_interpreter_home_env() -> io::Result<PathBuf> {
    let resolved = current_interpreter_home()?;
    std::fs::create_dir_all(&resolved)?;
    let fresh_home_provider_onboarding_marker =
        resolved.join(FRESH_HOME_PROVIDER_ONBOARDING_MARKER_FILE);
    let force_provider_onboarding = std::env::var_os(INTERPRETER_FORCE_PROVIDER_ONBOARDING_ENV_VAR)
        .is_some_and(|value| !value.is_empty())
        || fresh_home_provider_onboarding_marker.exists()
        || (!resolved.join(CONFIG_TOML_FILE).exists() && !resolved.join(AUTH_JSON_FILE).exists());
    record_startup_trace_event(if force_provider_onboarding {
        "interpreter.home.force_provider_onboarding.true"
    } else {
        "interpreter.home.force_provider_onboarding.false"
    });
    let canonical = resolved.canonicalize()?;
    if std::env::var_os(INTERPRETER_DISABLE_SYSTEM_IMPORT_ENV_VAR)
        .is_none_or(|value| value.is_empty())
    {
        crate::system_import::import_system_state(&canonical)?;
    }
    if force_provider_onboarding {
        let _ = std::fs::write(
            canonical.join(FRESH_HOME_PROVIDER_ONBOARDING_MARKER_FILE),
            "pending\n",
        );
    }
    // SAFETY: main() calls this before the tokio runtime starts any background
    // threads, so mutating the process environment here is safe.
    unsafe {
        std::env::set_var(CODEX_HOME_ENV_VAR, &canonical);
        std::env::set_var(INTERPRETER_HOME_ENV_VAR, &canonical);
        std::env::set_var(OPEN_INTERPRETER_HOME_ENV_VAR, &canonical);
        std::env::set_var(OPEN_INTERPRETER_BRAND_ENV_VAR, "1");
        if force_provider_onboarding {
            std::env::set_var(INTERPRETER_FORCE_PROVIDER_ONBOARDING_ENV_VAR, "1");
        } else {
            std::env::remove_var(INTERPRETER_FORCE_PROVIDER_ONBOARDING_ENV_VAR);
        }
    }
    Ok(canonical)
}

pub fn current_interpreter_home() -> io::Result<PathBuf> {
    resolve_interpreter_home_from_env(
        std::env::var_os(INTERPRETER_HOME_ENV_VAR).as_deref(),
        std::env::var_os(OPEN_INTERPRETER_HOME_ENV_VAR).as_deref(),
        fallback_home_directory(),
    )
}

fn resolve_interpreter_home_from_env(
    interpreter_home: Option<&OsStr>,
    open_interpreter_home: Option<&OsStr>,
    fallback_home_dir: Option<PathBuf>,
) -> io::Result<PathBuf> {
    if let Some(path) = non_empty_path(interpreter_home) {
        return Ok(path);
    }

    if let Some(path) = non_empty_path(open_interpreter_home) {
        return Ok(path);
    }

    let Some(home_dir) = fallback_home_dir else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "Could not find a home directory for Open Interpreter",
        ));
    };

    Ok(home_dir.join(DEFAULT_OPEN_INTERPRETER_HOME_DIR))
}

fn non_empty_path(value: Option<&OsStr>) -> Option<PathBuf> {
    value.filter(|value| !value.is_empty()).map(PathBuf::from)
}

fn fallback_home_directory() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn resolve_prefers_interpreter_home() {
        let resolved = resolve_interpreter_home_from_env(
            Some(OsStr::new("/tmp/interpreter-home")),
            Some(OsStr::new("/tmp/open-interpreter-home")),
            Some(PathBuf::from("/Users/test")),
        )
        .expect("resolve INTERPRETER_HOME");

        assert_eq!(resolved, PathBuf::from("/tmp/interpreter-home"));
    }

    #[test]
    fn resolve_falls_back_to_open_interpreter_home() {
        let resolved = resolve_interpreter_home_from_env(
            /*interpreter_home*/ None,
            Some(OsStr::new("/tmp/open-interpreter-home")),
            Some(PathBuf::from("/Users/test")),
        )
        .expect("resolve OPEN_INTERPRETER_HOME");

        assert_eq!(resolved, PathBuf::from("/tmp/open-interpreter-home"));
    }

    #[test]
    fn resolve_defaults_to_dot_openinterpreter() {
        let resolved = resolve_interpreter_home_from_env(
            /*interpreter_home*/ None,
            /*open_interpreter_home*/ None,
            Some(PathBuf::from("/Users/test")),
        )
        .expect("resolve default home");

        assert_eq!(resolved, PathBuf::from("/Users/test/.openinterpreter"));
    }

    #[test]
    fn resolve_rejects_missing_home_directory() {
        let err = resolve_interpreter_home_from_env(
            /*interpreter_home*/ None, /*open_interpreter_home*/ None,
            /*fallback_home_dir*/ None,
        )
        .expect_err("missing home dir");

        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert!(err.to_string().contains("Open Interpreter"));
    }
}
