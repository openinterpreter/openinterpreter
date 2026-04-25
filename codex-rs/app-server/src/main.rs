use codex_app_server::run_main_from_cli_args;
use codex_arg0::Arg0DispatchPaths;
use codex_arg0::arg0_dispatch_or_else;

fn main() -> anyhow::Result<()> {
    arg0_dispatch_or_else(|arg0_paths: Arg0DispatchPaths| async move {
        run_main_from_cli_args(arg0_paths, std::env::args_os()).await?;
        Ok(())
    })
}
