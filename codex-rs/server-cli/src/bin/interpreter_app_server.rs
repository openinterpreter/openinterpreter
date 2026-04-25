use codex_app_server::run_main_from_cli_args;
use codex_arg0::Arg0DispatchPaths;
use codex_arg0::arg0_dispatch_or_else_current_thread;
use codex_server_cli::startup_trace::record_startup_trace_event;
use std::ffi::OsString;

fn main() -> anyhow::Result<()> {
    record_startup_trace_event("interpreter_app_server.main.enter");
    arg0_dispatch_or_else_current_thread(|arg0_paths: Arg0DispatchPaths| async move {
        record_startup_trace_event("interpreter_app_server.dispatch.enter");
        let cli_args =
            std::iter::once(OsString::from("codex-app-server")).chain(std::env::args_os().skip(1));
        run_main_from_cli_args(arg0_paths, cli_args).await
    })
}
