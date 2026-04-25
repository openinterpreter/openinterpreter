use serde::Serialize;
use std::fs::OpenOptions;
use std::io::Write;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

const STARTUP_TRACE_PATH_ENV_VAR: &str = "CODEX_TUI_STARTUP_TRACE_PATH";

#[derive(Serialize)]
struct StartupTraceEvent<'a> {
    event: &'a str,
    pid: u32,
    unix_time_ms: u128,
}

pub(crate) fn record_startup_trace_event(event: &str) {
    let Some(path) = std::env::var_os(STARTUP_TRACE_PATH_ENV_VAR).filter(|value| !value.is_empty())
    else {
        return;
    };

    let Ok(unix_time_ms) = SystemTime::now().duration_since(UNIX_EPOCH) else {
        return;
    };
    let trace_event = StartupTraceEvent {
        event,
        pid: std::process::id(),
        unix_time_ms: unix_time_ms.as_millis(),
    };
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    let Ok(line) = serde_json::to_string(&trace_event) else {
        return;
    };
    let _ = writeln!(file, "{line}");
}
