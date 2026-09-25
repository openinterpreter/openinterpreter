pub const OPEN_INTERPRETER_BRAND_ENV_VAR: &str = "OPEN_INTERPRETER_BRAND";
pub const OPEN_INTERPRETER_DISPLAY_NAME: &str = "Open Interpreter";
pub const OPEN_INTERPRETER_COMMAND_NAME: &str = "interpreter";
pub const OPEN_INTERPRETER_NON_INTERACTIVE_ENV_VAR: &str = "OPEN_INTERPRETER_NONINTERACTIVE";

/// Upstream Codex release whose client behavior is embedded in Open Interpreter.
///
/// Open Interpreter has an independent product version, so backend compatibility
/// checks must not interpret that product version as the embedded Codex version.
pub const OPEN_INTERPRETER_CODEX_COMPATIBILITY_VERSION: &str = "0.156.1";

const CODEX_RELEASE_NOTES_URL: &str = "https://github.com/openai/codex/releases/latest";
const OPEN_INTERPRETER_RELEASE_NOTES_URL: &str =
    "https://github.com/openinterpreter/openinterpreter/releases/latest";
const CODEX_LATEST_RELEASE_URL: &str = "https://api.github.com/repos/openai/codex/releases/latest";
const OPEN_INTERPRETER_LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/openinterpreter/openinterpreter/releases/latest";
const CODEX_INSTALLER_URL: &str = "https://chatgpt.com/codex/install.sh";
const OPEN_INTERPRETER_INSTALLER_URL: &str = "https://www.openinterpreter.com/install";
const CODEX_MEMORIES_DOCS_URL: &str = "https://developers.openai.com/codex/memories";
const OPEN_INTERPRETER_MEMORIES_DOCS_URL: &str =
    "https://www.openinterpreter.com/docs/terminal/memories";
const CODEX_MCP_DOCS_URL: &str = "https://developers.openai.com/codex/mcp";
const OPEN_INTERPRETER_MCP_DOCS_URL: &str = "https://www.openinterpreter.com/docs/terminal/mcp";
const CODEX_WINDOWS_SANDBOX_DOCS_URL: &str = "https://developers.openai.com/codex/windows";
const OPEN_INTERPRETER_WINDOWS_SANDBOX_DOCS_URL: &str =
    "https://www.openinterpreter.com/docs/terminal/configuration";
const CODEX_INSTALL_COMMAND: &str = "curl -fsSL https://chatgpt.com/codex/install.sh | sh";
const OPEN_INTERPRETER_INSTALL_COMMAND: &str = "\
curl -fsSL https://www.openinterpreter.com/install | sh";
const CODEX_STANDALONE_UNIX_UPDATE_COMMAND: &str =
    "curl -fsSL https://chatgpt.com/codex/install.sh | CODEX_NON_INTERACTIVE=1 sh";
const OPEN_INTERPRETER_STANDALONE_UNIX_UPDATE_COMMAND: &str = "\
curl -fsSL https://www.openinterpreter.com/install | OPEN_INTERPRETER_NONINTERACTIVE=1 sh";
const CODEX_STANDALONE_WINDOWS_UPDATE_COMMAND: &str =
    "$env:CODEX_NON_INTERACTIVE=1; irm https://chatgpt.com/codex/install.ps1 | iex";
const OPEN_INTERPRETER_STANDALONE_WINDOWS_UPDATE_COMMAND: &str = "\
$env:OPEN_INTERPRETER_NONINTERACTIVE=1; \
irm https://www.openinterpreter.com/install.ps1 | iex";
const CODEX_STANDALONE_UNIX_UPDATE_ARGS: &[&str] = &["-c", CODEX_STANDALONE_UNIX_UPDATE_COMMAND];
const OPEN_INTERPRETER_STANDALONE_UNIX_UPDATE_ARGS: &[&str] =
    &["-c", OPEN_INTERPRETER_STANDALONE_UNIX_UPDATE_COMMAND];
const CODEX_STANDALONE_WINDOWS_UPDATE_ARGS: &[&str] = &[
    "-ExecutionPolicy",
    "Bypass",
    "-c",
    CODEX_STANDALONE_WINDOWS_UPDATE_COMMAND,
];
const OPEN_INTERPRETER_STANDALONE_WINDOWS_UPDATE_ARGS: &[&str] = &[
    "-ExecutionPolicy",
    "Bypass",
    "-c",
    OPEN_INTERPRETER_STANDALONE_WINDOWS_UPDATE_COMMAND,
];

/// Product channel information used by branded package variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Product {
    Codex,
    OpenInterpreter,
}

impl Product {
    /// A binary shipped inside an Open Interpreter package is Open
    /// Interpreter unconditionally — identity must never depend on what the
    /// executable happens to be named or aliased to. The argv0 and env-var
    /// checks only exist for development builds run straight out of the
    /// cargo target directory.
    pub fn current() -> Self {
        if std::env::var_os(OPEN_INTERPRETER_BRAND_ENV_VAR).is_some()
            || is_open_interpreter_argv0()
            || is_open_interpreter_install()
        {
            Self::OpenInterpreter
        } else {
            Self::Codex
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Product::Codex => "OpenAI Codex",
            Product::OpenInterpreter => OPEN_INTERPRETER_DISPLAY_NAME,
        }
    }

    pub fn short_display_name(self) -> &'static str {
        match self {
            Product::Codex => "Codex",
            Product::OpenInterpreter => OPEN_INTERPRETER_DISPLAY_NAME,
        }
    }

    pub fn command_name(self) -> &'static str {
        match self {
            Product::Codex => "codex",
            Product::OpenInterpreter => OPEN_INTERPRETER_COMMAND_NAME,
        }
    }

    /// Version advertised to Codex services for client compatibility checks.
    pub fn codex_compatibility_version(self) -> &'static str {
        match self {
            Product::Codex => env!("CARGO_PKG_VERSION"),
            Product::OpenInterpreter => OPEN_INTERPRETER_CODEX_COMPATIBILITY_VERSION,
        }
    }

    pub fn release_notes_url(self) -> &'static str {
        match self {
            Product::Codex => CODEX_RELEASE_NOTES_URL,
            Product::OpenInterpreter => OPEN_INTERPRETER_RELEASE_NOTES_URL,
        }
    }

    pub fn latest_release_url(self) -> &'static str {
        match self {
            Product::Codex => CODEX_LATEST_RELEASE_URL,
            Product::OpenInterpreter => OPEN_INTERPRETER_LATEST_RELEASE_URL,
        }
    }

    pub fn installer_url(self) -> &'static str {
        match self {
            Product::Codex => CODEX_INSTALLER_URL,
            Product::OpenInterpreter => OPEN_INTERPRETER_INSTALLER_URL,
        }
    }

    pub fn memories_docs_url(self) -> &'static str {
        match self {
            Product::Codex => CODEX_MEMORIES_DOCS_URL,
            Product::OpenInterpreter => OPEN_INTERPRETER_MEMORIES_DOCS_URL,
        }
    }

    pub fn mcp_docs_url(self) -> &'static str {
        match self {
            Product::Codex => CODEX_MCP_DOCS_URL,
            Product::OpenInterpreter => OPEN_INTERPRETER_MCP_DOCS_URL,
        }
    }

    pub fn windows_sandbox_docs_url(self) -> &'static str {
        match self {
            Product::Codex => CODEX_WINDOWS_SANDBOX_DOCS_URL,
            Product::OpenInterpreter => OPEN_INTERPRETER_WINDOWS_SANDBOX_DOCS_URL,
        }
    }

    pub fn install_command(self) -> &'static str {
        match self {
            Product::Codex => CODEX_INSTALL_COMMAND,
            Product::OpenInterpreter => OPEN_INTERPRETER_INSTALL_COMMAND,
        }
    }

    pub fn installer_env(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Product::Codex => &[],
            Product::OpenInterpreter => &[(OPEN_INTERPRETER_NON_INTERACTIVE_ENV_VAR, "1")],
        }
    }

    pub fn standalone_unix_update_args(self) -> &'static [&'static str] {
        match self {
            Product::Codex => CODEX_STANDALONE_UNIX_UPDATE_ARGS,
            Product::OpenInterpreter => OPEN_INTERPRETER_STANDALONE_UNIX_UPDATE_ARGS,
        }
    }

    pub fn standalone_windows_update_args(self) -> &'static [&'static str] {
        match self {
            Product::Codex => CODEX_STANDALONE_WINDOWS_UPDATE_ARGS,
            Product::OpenInterpreter => OPEN_INTERPRETER_STANDALONE_WINDOWS_UPDATE_ARGS,
        }
    }
}

pub fn is_open_interpreter_argv0() -> bool {
    std::env::args_os()
        .next()
        .and_then(|arg0| {
            std::path::Path::new(&arg0)
                .file_stem()
                .and_then(|name| name.to_str())
                .map(str::to_owned)
        })
        .is_some_and(|name| name.starts_with("interpreter") || name == "i")
}

/// Whether the running executable lives inside an installed Open Interpreter
/// package (bin/<exe> next to the package's codex-package.json metadata).
fn is_open_interpreter_install() -> bool {
    static IS_OPEN_INTERPRETER_INSTALL: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *IS_OPEN_INTERPRETER_INSTALL.get_or_init(|| {
        std::env::current_exe()
            .ok()
            // current_exe can return the path the binary was invoked through
            // (for example a symlink elsewhere); resolve to the real location
            // so the package metadata next to the binary decides.
            .map(|exe| exe.canonicalize().unwrap_or(exe))
            .is_some_and(|exe| executable_is_in_open_interpreter_package(&exe))
    })
}

fn executable_is_in_open_interpreter_package(exe: &std::path::Path) -> bool {
    let Some(package_dir) = exe.parent().and_then(std::path::Path::parent) else {
        return false;
    };
    let Ok(metadata) = std::fs::read_to_string(package_dir.join("codex-package.json")) else {
        return false;
    };
    metadata
        .split('"')
        .collect::<Vec<_>>()
        .windows(3)
        .any(|window| window[0] == "variant" && window[2] == "open-interpreter")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_inside_open_interpreter_package_is_open_interpreter() {
        let package = tempfile::tempdir().expect("tempdir");
        let bin_dir = package.path().join("bin");
        std::fs::create_dir_all(&bin_dir).expect("bin dir");
        let exe = bin_dir.join("codex");
        std::fs::write(&exe, b"").expect("exe");

        assert!(!executable_is_in_open_interpreter_package(&exe));

        std::fs::write(
            package.path().join("codex-package.json"),
            br#"{ "layoutVersion": 1, "variant": "open-interpreter", "entrypoint": "bin/interpreter" }"#,
        )
        .expect("metadata");
        assert!(executable_is_in_open_interpreter_package(&exe));

        std::fs::write(
            package.path().join("codex-package.json"),
            br#"{ "layoutVersion": 1, "variant": "codex", "entrypoint": "bin/codex" }"#,
        )
        .expect("metadata");
        assert!(!executable_is_in_open_interpreter_package(&exe));
    }

    #[test]
    fn codex_product_uses_upstream_release_channel() {
        assert_eq!(
            Product::Codex.release_notes_url(),
            "https://github.com/openai/codex/releases/latest"
        );
        assert_eq!(
            Product::Codex.latest_release_url(),
            "https://api.github.com/repos/openai/codex/releases/latest"
        );
        assert_eq!(
            Product::Codex.installer_url(),
            "https://chatgpt.com/codex/install.sh"
        );
        assert_eq!(
            Product::Codex.install_command(),
            "curl -fsSL https://chatgpt.com/codex/install.sh | sh"
        );
        assert_eq!(Product::Codex.installer_env(), &[]);
        assert_eq!(
            Product::Codex.codex_compatibility_version(),
            env!("CARGO_PKG_VERSION")
        );
    }

    #[test]
    fn open_interpreter_product_uses_openinterpreter_release_channel() {
        assert_eq!(
            Product::OpenInterpreter.release_notes_url(),
            "https://github.com/openinterpreter/openinterpreter/releases/latest"
        );
        assert_eq!(
            Product::OpenInterpreter.latest_release_url(),
            "https://api.github.com/repos/openinterpreter/openinterpreter/releases/latest"
        );
        assert_eq!(
            Product::OpenInterpreter.installer_url(),
            "https://www.openinterpreter.com/install"
        );
        assert_eq!(
            Product::OpenInterpreter.codex_compatibility_version(),
            OPEN_INTERPRETER_CODEX_COMPATIBILITY_VERSION
        );
        assert!(
            Product::OpenInterpreter
                .install_command()
                .contains("https://www.openinterpreter.com/install")
        );
        assert_eq!(
            Product::OpenInterpreter.installer_env(),
            &[(OPEN_INTERPRETER_NON_INTERACTIVE_ENV_VAR, "1")]
        );
        assert!(
            Product::OpenInterpreter
                .standalone_unix_update_args()
                .join(" ")
                .contains("OPEN_INTERPRETER_NONINTERACTIVE=1")
        );
        assert!(
            Product::OpenInterpreter
                .standalone_windows_update_args()
                .join(" ")
                .contains("OPEN_INTERPRETER_NONINTERACTIVE=1")
        );
    }
}
