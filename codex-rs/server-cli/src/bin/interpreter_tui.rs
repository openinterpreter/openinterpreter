use clap::Parser;
use codex_arg0::Arg0DispatchPaths;
use codex_arg0::arg0_dispatch_or_else_current_thread;
use codex_server_cli::home::ensure_interpreter_home_env;
use codex_server_cli::startup_trace::record_startup_trace_event;

fn main() -> anyhow::Result<()> {
    record_startup_trace_event("interpreter.main.enter");
    ensure_interpreter_home_env()?;
    record_startup_trace_event("interpreter.main.home.ready");
    arg0_dispatch_or_else_current_thread(|arg0_paths: Arg0DispatchPaths| async move {
        let cli = codex_server_cli::cli::ServerCli::parse();
        record_startup_trace_event("interpreter.main.cli.parsed");
        codex_server_cli::run::run_main(cli, arg0_paths).await
    })
}
