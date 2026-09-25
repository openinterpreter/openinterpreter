#[cfg(any(not(debug_assertions), test))]
use codex_install_context::InstallContext;
#[cfg(any(not(debug_assertions), test))]
use codex_install_context::InstallMethod;
#[cfg(any(not(debug_assertions), test))]
use codex_install_context::StandalonePlatform;

pub(crate) type ProductUpdateSource = codex_product_info::Product;

/// Update action the CLI should perform after the TUI exits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateAction {
    /// Replace the local daemon after restoring the terminal.
    Daemon(DaemonUpdateSource),
    /// Update via `npm install -g @openai/codex@latest`.
    NpmGlobalLatest,
    /// Update via `bun install -g @openai/codex@latest`.
    BunGlobalLatest,
    /// Update via `vp install -g @openai/codex@latest`.
    VitePlusGlobalLatest,
    /// Update via `pnpm add -g @openai/codex@latest`.
    PnpmGlobalLatest,
    /// Update via `brew upgrade codex`.
    BrewUpgrade,
    /// Update via `curl -fsSL https://chatgpt.com/codex/install.sh | CODEX_NON_INTERACTIVE=1 sh`.
    StandaloneUnix,
    /// Update via `$env:CODEX_NON_INTERACTIVE=1; irm https://chatgpt.com/codex/install.ps1 | iex`.
    StandaloneWindows,
}

impl UpdateAction {
    #[cfg(any(not(debug_assertions), test))]
    pub(crate) fn from_install_context(context: &InstallContext) -> Option<Self> {
        Self::from_install_context_for_product(codex_product_info::Product::current(), context)
    }

    #[cfg(any(not(debug_assertions), test))]
    fn from_install_context_for_product(
        product: codex_product_info::Product,
        context: &InstallContext,
    ) -> Option<Self> {
        let is_codex = product == codex_product_info::Product::Codex;
        match &context.method {
            // npm, bun, pnpm, and Homebrew distribute Codex; Open Interpreter ships
            // only the standalone package and must never offer to "update" by
            // installing OpenAI's Codex.
            InstallMethod::Npm => is_codex.then_some(UpdateAction::NpmGlobalLatest),
            InstallMethod::Bun => is_codex.then_some(UpdateAction::BunGlobalLatest),
            InstallMethod::VitePlus => is_codex.then_some(UpdateAction::VitePlusGlobalLatest),
            InstallMethod::Pnpm => is_codex.then_some(UpdateAction::PnpmGlobalLatest),
            InstallMethod::Brew => is_codex.then_some(UpdateAction::BrewUpgrade),
            InstallMethod::Standalone { platform, .. } => Some(match platform {
                StandalonePlatform::Unix => UpdateAction::StandaloneUnix,
                StandalonePlatform::Windows => UpdateAction::StandaloneWindows,
            }),
            InstallMethod::Other => None,
        }
    }

    /// Returns the list of command-line arguments for invoking the update.
    pub fn command_args(self) -> (&'static str, &'static [&'static str]) {
        self.command_args_for_source(ProductUpdateSource::current())
    }

    pub(crate) fn command_args_for_source(
        self,
        source: ProductUpdateSource,
    ) -> (&'static str, &'static [&'static str]) {
        match self {
            UpdateAction::Daemon(source) => ("codex", source.command_args()),
            UpdateAction::NpmGlobalLatest => ("npm", &["install", "-g", "@openai/codex"]),
            UpdateAction::BunGlobalLatest => ("bun", &["install", "-g", "@openai/codex"]),
            UpdateAction::VitePlusGlobalLatest => ("vp", &["install", "-g", "@openai/codex"]),
            UpdateAction::PnpmGlobalLatest => ("pnpm", &["add", "-g", "@openai/codex"]),
            UpdateAction::BrewUpgrade => ("brew", &["upgrade", "--cask", "codex"]),
            UpdateAction::StandaloneUnix => ("sh", source.standalone_unix_update_args()),
            UpdateAction::StandaloneWindows => {
                ("powershell", source.standalone_windows_update_args())
            }
        }
    }

    /// Returns string representation of the command-line arguments for invoking the update.
    pub fn command_str(self) -> String {
        let (command, args) = self.command_args();
        shlex::try_join(std::iter::once(command).chain(args.iter().copied()))
            .unwrap_or_else(|_| format!("{command} {}", args.join(" ")))
    }
}

#[cfg(not(debug_assertions))]
pub fn get_update_action() -> Option<UpdateAction> {
    UpdateAction::from_install_context(InstallContext::current())
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_utils_absolute_path::AbsolutePathBuf;
    use pretty_assertions::assert_eq;

    #[test]
    fn open_interpreter_never_updates_through_codex_channels() {
        for method in [InstallMethod::Npm, InstallMethod::Bun, InstallMethod::Brew] {
            assert_eq!(
                UpdateAction::from_install_context_for_product(
                    codex_product_info::Product::OpenInterpreter,
                    &InstallContext {
                        method,
                        package_layout: None,
                    }
                ),
                None
            );
        }
        let release_dir =
            AbsolutePathBuf::from_absolute_path(std::env::temp_dir().join("oi-release"))
                .expect("temp dir path should be absolute");
        assert_eq!(
            UpdateAction::from_install_context_for_product(
                codex_product_info::Product::OpenInterpreter,
                &InstallContext {
                    method: InstallMethod::Standalone {
                        release_dir,
                        resources_dir: None,
                        platform: StandalonePlatform::Unix,
                    },
                    package_layout: None,
                }
            ),
            Some(UpdateAction::StandaloneUnix)
        );
    }

    #[test]
    fn maps_install_context_to_update_action() {
        let native_release_dir =
            AbsolutePathBuf::from_absolute_path(std::env::temp_dir().join("native-release"))
                .expect("temp dir path should be absolute");

        assert_eq!(
            UpdateAction::from_install_context(&InstallContext {
                method: InstallMethod::Other,
                package_layout: None,
            }),
            None
        );
        assert_eq!(
            UpdateAction::from_install_context(&InstallContext {
                method: InstallMethod::Npm,
                package_layout: None,
            }),
            Some(UpdateAction::NpmGlobalLatest)
        );
        assert_eq!(
            UpdateAction::from_install_context(&InstallContext {
                method: InstallMethod::Bun,
                package_layout: None,
            }),
            Some(UpdateAction::BunGlobalLatest)
        );
        assert_eq!(
            UpdateAction::from_install_context(&InstallContext {
                method: InstallMethod::Pnpm,
                package_layout: None,
            }),
            Some(UpdateAction::PnpmGlobalLatest)
        );
        assert_eq!(
            UpdateAction::from_install_context(&InstallContext {
                method: InstallMethod::Brew,
                package_layout: None,
            }),
            Some(UpdateAction::BrewUpgrade)
        );
        assert_eq!(
            UpdateAction::from_install_context(&InstallContext {
                method: InstallMethod::Standalone {
                    platform: StandalonePlatform::Unix,
                    release_dir: native_release_dir.clone(),
                    resources_dir: Some(native_release_dir.join("codex-resources")),
                },
                package_layout: None,
            }),
            Some(UpdateAction::StandaloneUnix)
        );
        assert_eq!(
            UpdateAction::from_install_context(&InstallContext {
                method: InstallMethod::Standalone {
                    platform: StandalonePlatform::Windows,
                    release_dir: native_release_dir.clone(),
                    resources_dir: Some(native_release_dir.join("codex-resources")),
                },
                package_layout: None,
            }),
            Some(UpdateAction::StandaloneWindows)
        );
    }

    #[test]
    fn standalone_update_commands_rerun_latest_installer() {
        assert_eq!(
            UpdateAction::StandaloneUnix.command_args(),
            (
                "sh",
                &[
                    "-c",
                    "curl -fsSL https://chatgpt.com/codex/install.sh | CODEX_NON_INTERACTIVE=1 sh"
                ][..],
            )
        );
        assert_eq!(
            UpdateAction::StandaloneWindows.command_args(),
            (
                "powershell",
                &[
                    "-ExecutionPolicy",
                    "Bypass",
                    "-c",
                    "$env:CODEX_NON_INTERACTIVE=1; irm https://chatgpt.com/codex/install.ps1 | iex"
                ][..],
            )
        );
    }

    #[test]
    fn open_interpreter_standalone_update_commands_rerun_latest_installer() {
        assert_eq!(
            UpdateAction::StandaloneUnix
                .command_args_for_source(ProductUpdateSource::OpenInterpreter),
            (
                "sh",
                ProductUpdateSource::OpenInterpreter.standalone_unix_update_args(),
            )
        );
        assert_eq!(
            UpdateAction::StandaloneWindows
                .command_args_for_source(ProductUpdateSource::OpenInterpreter),
            (
                "powershell",
                ProductUpdateSource::OpenInterpreter.standalone_windows_update_args(),
            )
        );
    }

    #[test]
    fn update_source_urls_follow_product() {
        assert_eq!(
            ProductUpdateSource::Codex.release_notes_url(),
            "https://github.com/openai/codex/releases/latest"
        );
        assert_eq!(
            ProductUpdateSource::OpenInterpreter.release_notes_url(),
            "https://github.com/openinterpreter/openinterpreter/releases/latest"
        );
        assert_eq!(
            ProductUpdateSource::Codex.latest_release_url(),
            "https://api.github.com/repos/openai/codex/releases/latest"
        );
        assert_eq!(
            ProductUpdateSource::OpenInterpreter.latest_release_url(),
            "https://api.github.com/repos/openinterpreter/openinterpreter/releases/latest"
        );
    }
}

/// Package source explicitly selected by the user in the daemon menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonUpdateSource {
    PublicStable,
    ThisCli,
}

impl DaemonUpdateSource {
    pub fn command_args(self) -> &'static [&'static str] {
        match self {
            Self::PublicStable => &["app-server", "daemon", "update"],
            Self::ThisCli => &["app-server", "daemon", "update", "--from-cli", "--yes"],
        }
    }
}
