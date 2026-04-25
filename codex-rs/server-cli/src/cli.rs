pub use crate::cli_common::AltScreenCli;
use crate::cli_common::FeatureToggles;
pub use crate::cli_common::KillCommand;
pub use crate::cli_common::LaunchOptions;
pub use crate::cli_common::daemon_startup_overrides;
use clap::Args;
use clap::FromArgMatches;
use clap::Parser;
use codex_tui::Cli as TuiCli;
use codex_utils_cli::CliConfigOverrides;
use std::ffi::OsString;

#[derive(Parser, Debug)]
#[command(
    name = "interpreter",
    about = "Open Interpreter app-server-backed TUI",
    version,
    subcommand_negates_reqs = true,
    override_usage = "interpreter [OPTIONS] [PROMPT]\n       interpreter [OPTIONS] <COMMAND> [ARGS]"
)]
pub struct ServerCli {
    #[clap(flatten)]
    pub config_overrides: CliConfigOverrides,

    #[clap(flatten)]
    pub feature_toggles: FeatureToggles,

    #[clap(flatten)]
    pub launch: LaunchOptions,

    #[command(flatten)]
    pub alt_screen: AltScreenCli,

    #[command(flatten)]
    pub interactive: TuiCli,

    #[command(subcommand)]
    pub subcommand: Option<Subcommand>,
}

#[derive(Debug, clap::Subcommand)]
pub enum Subcommand {
    /// Resume a previous interactive session (picker by default; use --last to continue the most recent).
    Resume(ResumeCommand),

    /// Fork a previous interactive session (picker by default; use --last to fork the most recent).
    Fork(ForkCommand),

    /// Run Open Interpreter non-interactively through the app-server daemon.
    #[clap(visible_alias = "e")]
    Exec(ExecCommand),

    /// Stop the local Open Interpreter daemon.
    Kill(KillCommand),
}

#[derive(Args, Debug)]
struct ResumeCommandRaw {
    /// Conversation/session id (UUID) or thread name. UUIDs take precedence if it parses.
    /// If omitted, use --last to pick the most recent recorded session.
    #[arg(value_name = "SESSION_ID")]
    session_id: Option<String>,

    /// Continue the most recent session without showing the picker.
    #[arg(long = "last", default_value_t = false)]
    last: bool,

    /// Show all sessions (disables cwd filtering and shows CWD column).
    #[arg(long = "all", default_value_t = false)]
    all: bool,

    /// Include non-interactive sessions in the resume picker and --last selection.
    #[arg(long = "include-non-interactive", default_value_t = false)]
    include_non_interactive: bool,

    #[clap(flatten)]
    launch: LaunchOptions,

    #[clap(flatten)]
    interactive: TuiCli,
}

#[derive(Debug)]
pub struct ResumeCommand {
    pub session_id: Option<String>,
    pub last: bool,
    pub all: bool,
    pub include_non_interactive: bool,
    pub launch: LaunchOptions,
    pub interactive: TuiCli,
}

impl From<ResumeCommandRaw> for ResumeCommand {
    fn from(raw: ResumeCommandRaw) -> Self {
        let mut interactive = raw.interactive;
        let (session_id, prompt) = if raw.last && interactive.prompt.is_none() {
            (None, raw.session_id)
        } else {
            (raw.session_id, interactive.prompt.take())
        };
        interactive.prompt = prompt;
        Self {
            session_id,
            last: raw.last,
            all: raw.all,
            include_non_interactive: raw.include_non_interactive,
            launch: raw.launch,
            interactive,
        }
    }
}

impl Args for ResumeCommand {
    fn augment_args(cmd: clap::Command) -> clap::Command {
        ResumeCommandRaw::augment_args(cmd)
    }

    fn augment_args_for_update(cmd: clap::Command) -> clap::Command {
        ResumeCommandRaw::augment_args_for_update(cmd)
    }
}

impl FromArgMatches for ResumeCommand {
    fn from_arg_matches(matches: &clap::ArgMatches) -> Result<Self, clap::Error> {
        ResumeCommandRaw::from_arg_matches(matches).map(Self::from)
    }

    fn update_from_arg_matches(&mut self, matches: &clap::ArgMatches) -> Result<(), clap::Error> {
        *self = ResumeCommandRaw::from_arg_matches(matches).map(Self::from)?;
        Ok(())
    }
}

#[derive(Debug, Parser)]
pub struct ForkCommand {
    /// Conversation/session id (UUID). When provided, forks this session.
    /// If omitted, use --last to pick the most recent recorded session.
    #[arg(value_name = "SESSION_ID")]
    pub session_id: Option<String>,

    /// Fork the most recent session without showing the picker.
    #[arg(long = "last", default_value_t = false, conflicts_with = "session_id")]
    pub last: bool,

    /// Show all sessions (disables cwd filtering and shows CWD column).
    #[arg(long = "all", default_value_t = false)]
    pub all: bool,

    #[clap(flatten)]
    pub launch: LaunchOptions,

    #[clap(flatten)]
    pub interactive: TuiCli,
}

#[derive(Debug, Parser)]
pub struct ExecCommand {
    #[arg(
        trailing_var_arg = true,
        allow_hyphen_values = true,
        value_name = "ARGS"
    )]
    pub args: Vec<OsString>,
}

pub fn apply_root_overrides(
    mut interactive: TuiCli,
    root_config_overrides: CliConfigOverrides,
) -> TuiCli {
    prepend_config_flags(&mut interactive.config_overrides, root_config_overrides);
    interactive
}

pub fn finalize_resume_interactive(
    mut interactive: TuiCli,
    root_config_overrides: CliConfigOverrides,
    session_id: Option<String>,
    last: bool,
    show_all: bool,
    include_non_interactive: bool,
    resume_cli: TuiCli,
) -> TuiCli {
    let resume_session_id = session_id;
    interactive.resume_picker = resume_session_id.is_none() && !last;
    interactive.resume_last = last;
    interactive.resume_session_id = resume_session_id;
    interactive.resume_show_all = show_all;
    interactive.resume_include_non_interactive = include_non_interactive;

    merge_interactive_cli_flags(&mut interactive, resume_cli);
    prepend_config_flags(&mut interactive.config_overrides, root_config_overrides);

    interactive
}

pub fn finalize_fork_interactive(
    mut interactive: TuiCli,
    root_config_overrides: CliConfigOverrides,
    session_id: Option<String>,
    last: bool,
    show_all: bool,
    fork_cli: TuiCli,
) -> TuiCli {
    let fork_session_id = session_id;
    interactive.fork_picker = fork_session_id.is_none() && !last;
    interactive.fork_last = last;
    interactive.fork_session_id = fork_session_id;
    interactive.fork_show_all = show_all;

    merge_interactive_cli_flags(&mut interactive, fork_cli);
    prepend_config_flags(&mut interactive.config_overrides, root_config_overrides);

    interactive
}

fn prepend_config_flags(
    subcommand_config_overrides: &mut CliConfigOverrides,
    cli_config_overrides: CliConfigOverrides,
) {
    subcommand_config_overrides
        .raw_overrides
        .splice(0..0, cli_config_overrides.raw_overrides);
}

fn merge_interactive_cli_flags(interactive: &mut TuiCli, subcommand_cli: TuiCli) {
    if let Some(model) = subcommand_cli.model {
        interactive.model = Some(model);
    }
    if subcommand_cli.oss {
        interactive.oss = true;
    }
    if let Some(oss_provider) = subcommand_cli.oss_provider {
        interactive.oss_provider = Some(oss_provider);
    }
    if let Some(profile) = subcommand_cli.config_profile {
        interactive.config_profile = Some(profile);
    }
    if let Some(sandbox) = subcommand_cli.sandbox_mode {
        interactive.sandbox_mode = Some(sandbox);
    }
    if let Some(approval) = subcommand_cli.approval_policy {
        interactive.approval_policy = Some(approval);
    }
    if subcommand_cli.full_auto {
        interactive.full_auto = true;
    }
    if subcommand_cli.dangerously_bypass_approvals_and_sandbox {
        interactive.dangerously_bypass_approvals_and_sandbox = true;
    }
    if let Some(cwd) = subcommand_cli.cwd {
        interactive.cwd = Some(cwd);
    }
    if subcommand_cli.web_search {
        interactive.web_search = true;
    }
    if !subcommand_cli.images.is_empty() {
        interactive.images = subcommand_cli.images;
    }
    if !subcommand_cli.add_dir.is_empty() {
        interactive.add_dir.extend(subcommand_cli.add_dir);
    }
    if subcommand_cli.no_alt_screen {
        interactive.no_alt_screen = true;
    }
    if let Some(prompt) = subcommand_cli.prompt {
        interactive.prompt = Some(prompt.replace("\r\n", "\n").replace('\r', "\n"));
    }

    interactive
        .config_overrides
        .raw_overrides
        .extend(subcommand_cli.config_overrides.raw_overrides);
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    use pretty_assertions::assert_eq;

    #[test]
    fn forwards_feature_toggles_into_config_overrides() {
        let cli = ServerCli::parse_from([
            "interpreter",
            "--enable",
            "foo",
            "--disable",
            "bar",
            "-c",
            "model=\"gpt-5.4\"",
        ]);

        let mut root_overrides = cli.config_overrides;
        root_overrides
            .raw_overrides
            .extend(cli.feature_toggles.into_overrides());
        let interactive = apply_root_overrides(cli.interactive, root_overrides);

        assert_eq!(
            interactive.config_overrides.raw_overrides,
            vec![
                "model=\"gpt-5.4\"".to_string(),
                "features.foo=true".to_string(),
                "features.bar=false".to_string(),
            ]
        );
    }

    #[test]
    fn daemon_startup_overrides_do_not_reintroduce_removed_defaults() {
        let overrides = CliConfigOverrides {
            raw_overrides: vec![
                "model=\"gpt-5.4\"".to_string(),
                "features.foo=true".to_string(),
            ],
        };

        assert_eq!(daemon_startup_overrides(&overrides), Vec::<String>::new());
    }

    #[test]
    fn daemon_startup_overrides_keep_only_daemon_safe_feature_overrides() {
        let overrides = CliConfigOverrides {
            raw_overrides: vec![
                "features.apps=false".to_string(),
                "features.plugins=true".to_string(),
                "features.default_mode_request_user_input=true".to_string(),
                "model=\"gpt-5.4-mini\"".to_string(),
            ],
        };

        assert_eq!(
            daemon_startup_overrides(&overrides),
            vec![
                "features.apps=false".to_string(),
                "features.plugins=true".to_string(),
                "features.default_mode_request_user_input=true".to_string(),
            ]
        );
    }

    #[test]
    fn parses_remote_options_separately_from_tui_cli() {
        let cli = ServerCli::parse_from([
            "interpreter",
            "--remote",
            "ws://127.0.0.1:7777",
            "--remote-auth-token-env",
            "CODEX_TOKEN",
            "hello",
        ]);

        assert_eq!(cli.launch.remote, Some("ws://127.0.0.1:7777".to_string()));
        assert_eq!(
            cli.launch.remote_auth_token_env,
            Some("CODEX_TOKEN".to_string())
        );
        assert_eq!(cli.interactive.prompt, Some("hello".to_string()));
    }

    #[test]
    fn resume_subcommand_merges_root_and_subcommand_flags() {
        let cli = ServerCli::parse_from([
            "interpreter",
            "--enable",
            "foo",
            "--profile",
            "root-profile",
            "resume",
            "--last",
            "--profile",
            "resume-profile",
            "--search",
            "hello",
        ]);

        let ServerCli {
            config_overrides,
            feature_toggles,
            launch: _,
            alt_screen: _,
            interactive,
            subcommand,
        } = cli;
        let Some(Subcommand::Resume(resume)) = subcommand else {
            panic!("expected resume subcommand");
        };
        let mut root_overrides = config_overrides;
        root_overrides
            .raw_overrides
            .extend(feature_toggles.into_overrides());
        let interactive = finalize_resume_interactive(
            interactive,
            root_overrides,
            resume.session_id,
            resume.last,
            resume.all,
            resume.include_non_interactive,
            resume.interactive,
        );

        assert!(interactive.resume_last);
        assert_eq!(
            interactive.config_profile.as_deref(),
            Some("resume-profile")
        );
        assert!(interactive.web_search);
        assert_eq!(interactive.prompt.as_deref(), Some("hello"));
        assert_eq!(
            interactive.config_overrides.raw_overrides,
            vec!["features.foo=true".to_string(),]
        );
    }

    #[test]
    fn alt_screen_flag_parses_as_global_after_resume_subcommand() {
        let cli = ServerCli::parse_from(["interpreter", "resume", "--last", "--alt-screen"]);

        assert!(cli.alt_screen.alt_screen);
    }

    #[test]
    fn exec_subcommand_captures_trailing_args_verbatim() {
        let cli = ServerCli::parse_from([
            "interpreter",
            "exec",
            "--json",
            "--profile",
            "chat",
            "hello",
        ]);

        let Some(Subcommand::Exec(exec)) = cli.subcommand else {
            panic!("expected exec subcommand");
        };
        assert_eq!(
            exec.args,
            vec![
                OsString::from("--json"),
                OsString::from("--profile"),
                OsString::from("chat"),
                OsString::from("hello"),
            ]
        );
    }

    #[test]
    fn help_hides_internal_app_server_override() {
        let mut command = ServerCli::command();
        let mut help = Vec::new();
        command
            .write_long_help(&mut help)
            .expect("render interpreter help");
        let help = String::from_utf8(help).expect("help should be utf8");

        assert!(!help.contains("--app-server-bin"));
    }

    #[test]
    fn kill_subcommand_parses() {
        let cli = ServerCli::parse_from(["interpreter", "kill"]);

        let Some(Subcommand::Kill(kill)) = cli.subcommand else {
            panic!("expected kill subcommand");
        };
        assert_eq!(kill, KillCommand::default());
    }

    #[test]
    fn kill_subcommand_force_parses() {
        let cli = ServerCli::parse_from(["interpreter", "kill", "--force"]);

        let Some(Subcommand::Kill(kill)) = cli.subcommand else {
            panic!("expected kill subcommand");
        };
        assert_eq!(kill, KillCommand { force: true });
    }
}
