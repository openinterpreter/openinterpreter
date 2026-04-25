#![cfg(not(target_os = "windows"))]

mod common;

use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use anyhow::Context;
use anyhow::Result;
use codex_tui::STARTUP_TRACE_PATH_ENV_VAR;
use codex_utils_cargo_bin::repo_root;
use pretty_assertions::assert_eq;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Map;
use serde_json::Value;
use serde_json::json;
use tempfile::NamedTempFile;
use tempfile::TempDir;
use tokio::time::sleep;
use tokio::time::timeout;

use crate::common::MockResponsesServer;
use crate::common::TmuxSession;
use crate::common::is_prompt_visible_screen;
use crate::common::is_session_ready_screen;
use crate::common::resolve_codex_bin;
use crate::common::resolve_interpreter_bin;
use crate::common::tmux_is_available;

const DEFAULT_BENCH_CLIENT_COUNTS: &[usize] = &[1, 2, 5, 10];
const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);
const TURN_TIMEOUT: Duration = Duration::from_secs(180);
const TEST_MODEL: &str = "gpt-5.4-mini";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CliFlavor {
    Codex,
    Interpreter,
}

impl CliFlavor {
    fn label(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Interpreter => "interpreter",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BenchmarkProfile {
    Responses,
    Chat,
}

impl BenchmarkProfile {
    fn label(self) -> &'static str {
        match self {
            Self::Responses => "responses",
            Self::Chat => "chat",
        }
    }
}

struct HeldClient {
    label: String,
    pid: u32,
    session: TmuxSession,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct DaemonLockfile {
    pid: u32,
    websocket_url: String,
    server_bin: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
struct ProcessMemorySample {
    rss_kb: u64,
    phys_footprint_bytes: Option<u64>,
    phys_footprint_peak_bytes: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct StartupTraceEvent {
    event: String,
    pid: u32,
    unix_time_ms: u128,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
struct StartupMeasurement {
    ready_ms: u128,
    session_ready_ms: u128,
    pre_trace_gap_ms: Option<u128>,
    trace_span_ms: Option<u128>,
    post_trace_gap_ms: Option<u128>,
    trace: Vec<StartupTraceEvent>,
    phase_durations_ms: BTreeMap<String, u128>,
}

struct LaunchSessionSpec<'a> {
    flavor: CliFlavor,
    home: &'a Path,
    repo_root: &'a Path,
    api_key: Option<&'a str>,
    profile: Option<BenchmarkProfile>,
    log_dir: PathBuf,
    session_suffix: String,
    startup_trace_path: Option<&'a Path>,
}

#[cfg(target_os = "macos")]
#[derive(Debug, Deserialize)]
struct FootprintJson {
    processes: Vec<FootprintProcess>,
}

#[cfg(target_os = "macos")]
#[derive(Debug, Deserialize)]
struct FootprintProcess {
    pid: u32,
    auxiliary: Option<FootprintAuxiliary>,
}

#[cfg(target_os = "macos")]
#[derive(Debug, Deserialize)]
struct FootprintAuxiliary {
    phys_footprint: Option<u64>,
    phys_footprint_peak: Option<u64>,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "benchmark: compare cold and warm PTY startup times"]
async fn startup_benchmark_records_codex_and_interpreter_ready_times() -> Result<()> {
    if !tmux_is_available() {
        eprintln!("skipping startup benchmark because tmux is not available");
        return Ok(());
    }

    let responses_server =
        MockResponsesServer::start(TEST_MODEL, "STARTUP_BENCHMARK_OK", Duration::ZERO).await?;

    let repo_root = repo_root()?;
    let codex_home = TempDir::new()?;
    let interpreter_home = TempDir::new()?;
    write_mock_config(codex_home.path(), &repo_root, responses_server.base_url())?;
    write_mock_config(
        interpreter_home.path(),
        &repo_root,
        responses_server.base_url(),
    )?;

    let codex_cold = measure_ready_time(CliFlavor::Codex, codex_home.path(), &repo_root).await?;
    let codex_warm = measure_ready_time(CliFlavor::Codex, codex_home.path(), &repo_root).await?;
    let interpreter_cold =
        measure_ready_time(CliFlavor::Interpreter, interpreter_home.path(), &repo_root).await?;
    let interpreter_warm =
        measure_ready_time(CliFlavor::Interpreter, interpreter_home.path(), &repo_root).await?;

    wait_for_daemon_exit_if_present(interpreter_home.path()).await?;

    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "startup_ready_ms": {
                "codex": {
                    "cold": codex_cold,
                    "warm": codex_warm,
                },
                "interpreter": {
                    "cold": interpreter_cold,
                    "warm": interpreter_warm,
                },
            }
        }))?
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "benchmark: live OPENAI_API_KEY memory comparison across 10 multi-turn clients"]
async fn memory_benchmark_profiles_codex_and_interpreter_after_three_turns() -> Result<()> {
    if !tmux_is_available() {
        eprintln!("skipping memory benchmark because tmux is not available");
        return Ok(());
    }

    let Some(api_key) = std::env::var("OPENAI_API_KEY")
        .ok()
        .filter(|value| !value.is_empty())
    else {
        eprintln!("skipping live memory benchmark because OPENAI_API_KEY is not set");
        return Ok(());
    };

    let repo_root = repo_root()?;
    let client_counts = benchmark_client_counts()?;
    let flavors = benchmark_flavors()?;
    let profiles = benchmark_profiles()?;
    let mut profile_reports = Map::new();

    for profile in profiles {
        let mut count_reports = Map::new();
        for client_count in &client_counts {
            let mut flavor_reports = Map::new();
            for flavor in &flavors {
                let home = TempDir::new()?;
                write_live_config(home.path(), &repo_root)?;

                let report = benchmark_live_group(
                    *flavor,
                    home.path(),
                    &repo_root,
                    &api_key,
                    profile,
                    *client_count,
                )
                .await?;
                flavor_reports.insert(flavor.label().to_string(), json!(report));
            }

            count_reports.insert(client_count.to_string(), Value::Object(flavor_reports));
        }

        profile_reports.insert(profile.label().to_string(), Value::Object(count_reports));
    }

    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "client_counts": client_counts,
            "turns_per_client": 3,
            "profiles": profile_reports,
        }))?
    );

    Ok(())
}

async fn benchmark_live_group(
    flavor: CliFlavor,
    home: &Path,
    repo_root: &Path,
    api_key: &str,
    profile: BenchmarkProfile,
    client_count: usize,
) -> Result<Value> {
    let mut clients = Vec::with_capacity(client_count);
    for session_index in 0..client_count {
        clients.push(
            spawn_ready_session(
                flavor,
                home,
                repo_root,
                Some(api_key),
                Some(profile),
                home.join(format!("bench-{}-{session_index}", flavor.label())),
                session_index,
            )
            .await?,
        );
    }

    let prompts_by_client = (0..client_count).map(benchmark_prompts).collect::<Vec<_>>();
    let turn_count = prompts_by_client.first().map_or(0, Vec::len);

    for turn_index in 0..turn_count {
        for (client, prompts) in clients.iter().zip(&prompts_by_client) {
            submit_prompt_fast(&client.session, &prompts[turn_index].0)?;
        }

        for (client, prompts) in clients.iter().zip(&prompts_by_client) {
            wait_for_visible_text(
                &client.label,
                &client.session,
                &prompts[turn_index].1,
                TURN_TIMEOUT,
            )
            .await?;
        }

        sleep(Duration::from_millis(250)).await;
    }

    sleep(Duration::from_secs(2)).await;

    let client_pids = clients.iter().map(|client| client.pid).collect::<Vec<_>>();
    let client_memory = sample_process_memory(&client_pids)?;
    let mut report = json!({
        "profile": profile.label(),
        "client_count": client_count,
        "client_pids": client_pids,
        "client_memory": client_memory,
        "total_client_rss_kb": sum_rss_kb(&client_memory),
    });

    if let Some(total_client_phys_footprint_bytes) = sum_phys_footprint_bytes(&client_memory) {
        report["total_client_phys_footprint_bytes"] = json!(total_client_phys_footprint_bytes);
    }
    if let Some(total_client_phys_footprint_peak_bytes) =
        sum_phys_footprint_peak_bytes(&client_memory)
    {
        report["total_client_phys_footprint_peak_bytes"] =
            json!(total_client_phys_footprint_peak_bytes);
    }

    if matches!(flavor, CliFlavor::Interpreter) {
        let daemon = read_daemon_lockfile(home)?;
        let daemon_memory = sample_process_memory(&[daemon.pid])?;
        let daemon_sample = daemon_memory
            .get(&daemon.pid.to_string())
            .cloned()
            .with_context(|| format!("missing memory sample for daemon PID {}", daemon.pid))?;

        report["daemon_pid"] = json!(daemon.pid);
        report["daemon_websocket_url"] = json!(daemon.websocket_url);
        report["daemon_memory"] = json!(daemon_sample);
        report["total_rss_kb"] = json!(sum_rss_kb(&client_memory) + daemon_sample.rss_kb);

        if let Some(total_client_phys_footprint_bytes) = sum_phys_footprint_bytes(&client_memory)
            && let Some(daemon_phys_footprint_bytes) = daemon_sample.phys_footprint_bytes
        {
            report["total_phys_footprint_bytes"] =
                json!(total_client_phys_footprint_bytes + daemon_phys_footprint_bytes);
        }
        if let Some(total_client_phys_footprint_peak_bytes) =
            sum_phys_footprint_peak_bytes(&client_memory)
            && let Some(daemon_phys_footprint_peak_bytes) = daemon_sample.phys_footprint_peak_bytes
        {
            report["total_phys_footprint_peak_bytes"] =
                json!(total_client_phys_footprint_peak_bytes + daemon_phys_footprint_peak_bytes);
        }
    }

    drop(clients);
    sleep(Duration::from_secs(1)).await;

    if matches!(flavor, CliFlavor::Interpreter) {
        wait_for_daemon_exit_if_present(home).await?;
    }

    Ok(report)
}

async fn measure_ready_time(
    flavor: CliFlavor,
    home: &Path,
    repo_root: &Path,
) -> Result<StartupMeasurement> {
    match flavor {
        CliFlavor::Codex => {
            let _ = resolve_codex_bin()?;
        }
        CliFlavor::Interpreter => {
            let _ = resolve_interpreter_bin()?;
        }
    }
    let log_dir = home.join(format!("startup-{}", flavor.label()));
    let startup_trace = NamedTempFile::new()?;
    let measure_start_unix_time_ms = current_unix_time_ms()?;
    let start = Instant::now();
    let session = launch_session(LaunchSessionSpec {
        flavor,
        home,
        repo_root,
        api_key: None,
        profile: None,
        log_dir,
        session_suffix: "startup".to_string(),
        startup_trace_path: Some(startup_trace.path()),
    })
    .await?;
    wait_for_first_prompt_screen(flavor.label(), &session, STARTUP_TIMEOUT).await?;
    let ready = start.elapsed().as_millis();
    wait_for_session_ready_screen(flavor.label(), &session, STARTUP_TIMEOUT).await?;
    let session_ready_ms = start.elapsed().as_millis();
    drop(session);
    let trace = read_startup_trace(startup_trace.path())?;
    let pre_trace_gap_ms = trace.first().map(|event| {
        event
            .unix_time_ms
            .saturating_sub(measure_start_unix_time_ms)
    });
    let trace_span_ms = trace_span_duration(&trace);
    let post_trace_gap_ms = match (pre_trace_gap_ms, trace_span_ms) {
        (Some(pre_trace_gap_ms), Some(trace_span_ms)) => {
            Some(ready.saturating_sub(pre_trace_gap_ms + trace_span_ms))
        }
        _ => None,
    };
    Ok(StartupMeasurement {
        ready_ms: ready,
        session_ready_ms,
        pre_trace_gap_ms,
        trace_span_ms,
        post_trace_gap_ms,
        phase_durations_ms: startup_phase_durations(&trace),
        trace,
    })
}

async fn spawn_ready_session(
    flavor: CliFlavor,
    home: &Path,
    repo_root: &Path,
    api_key: Option<&str>,
    profile: Option<BenchmarkProfile>,
    log_dir: PathBuf,
    session_index: usize,
) -> Result<HeldClient> {
    let session = launch_session(LaunchSessionSpec {
        flavor,
        home,
        repo_root,
        api_key,
        profile,
        log_dir,
        session_suffix: format!("session-{session_index}"),
        startup_trace_path: None,
    })
    .await?;
    let pid = session.pane_pid()?;
    wait_for_session_ready_screen(flavor.label(), &session, STARTUP_TIMEOUT).await?;

    Ok(HeldClient {
        label: format!("{}-session-{session_index}", flavor.label()),
        pid,
        session,
    })
}

async fn launch_session(spec: LaunchSessionSpec<'_>) -> Result<TmuxSession> {
    let LaunchSessionSpec {
        flavor,
        home,
        repo_root,
        api_key,
        profile,
        log_dir,
        session_suffix,
        startup_trace_path,
    } = spec;
    let binary = match flavor {
        CliFlavor::Codex => resolve_codex_bin()?,
        CliFlavor::Interpreter => resolve_interpreter_bin()?,
    };
    let env = benchmark_env(flavor, home, api_key, startup_trace_path);
    let args = launch_args(flavor, repo_root, &log_dir, profile)?;
    TmuxSession::start(
        &format!("{}-{session_suffix}", flavor.label()),
        binary.as_path(),
        &args,
        repo_root,
        &env,
    )
}

async fn wait_for_first_prompt_screen(
    label: &str,
    session: &TmuxSession,
    timeout_duration: Duration,
) -> Result<String> {
    session
        .wait_for_screen(timeout_duration, is_prompt_visible_screen)
        .await
        .with_context(|| format!("timed out waiting for first prompt on {label}"))
}

async fn wait_for_session_ready_screen(
    label: &str,
    session: &TmuxSession,
    timeout_duration: Duration,
) -> Result<String> {
    session
        .wait_for_screen(timeout_duration, is_session_ready_screen)
        .await
        .with_context(|| format!("timed out waiting for session-ready screen on {label}"))
}

async fn wait_for_visible_text(
    label: &str,
    session: &TmuxSession,
    expected: &str,
    timeout_duration: Duration,
) -> Result<String> {
    session
        .wait_for_screen(timeout_duration, |pane| pane.contains(expected))
        .await
        .with_context(|| format!("timed out waiting for `{expected}` on {label}"))
}

fn read_startup_trace(path: &Path) -> Result<Vec<StartupTraceEvent>> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read startup trace {}", path.display()))?;
    let mut events = Vec::new();
    for line in content.lines().filter(|line| !line.trim().is_empty()) {
        events.push(
            serde_json::from_str(line)
                .with_context(|| format!("failed to parse startup trace line `{line}`"))?,
        );
    }
    Ok(events)
}

fn startup_phase_durations(trace: &[StartupTraceEvent]) -> BTreeMap<String, u128> {
    fn phase_duration(
        trace: &[StartupTraceEvent],
        start_event: &str,
        end_event: &str,
    ) -> Option<u128> {
        let start_time = trace
            .iter()
            .find(|event| event.event == start_event)
            .map(|event| event.unix_time_ms)?;
        let end_time = trace
            .iter()
            .find(|event| event.event == end_event)
            .map(|event| event.unix_time_ms)?;
        Some(end_time.saturating_sub(start_time))
    }

    let mut phases = BTreeMap::new();
    for (label, start_event, end_event) in [
        ("codex_main", "codex.main.enter", "codex.cli_main.enter"),
        (
            "codex_cli_parse",
            "codex.cli_main.enter",
            "codex.cli.parsed",
        ),
        (
            "codex_dispatch_to_tui",
            "codex.cli.parsed",
            "tui.run_main.enter",
        ),
        (
            "interpreter_main_home",
            "interpreter.main.enter",
            "interpreter.main.home.ready",
        ),
        (
            "interpreter_cli_parse",
            "interpreter.main.home.ready",
            "interpreter.main.cli.parsed",
        ),
        (
            "interpreter_dispatch_to_run_main",
            "interpreter.main.cli.parsed",
            "interpreter.run_main.enter",
        ),
        (
            "daemon_ensure",
            "interpreter.daemon.ensure.start",
            "interpreter.daemon.ensure.end",
        ),
        (
            "daemon_spawn",
            "interpreter.daemon.spawn.start",
            "interpreter.daemon.spawn.ready",
        ),
        (
            "config_toml_load",
            "tui.run_main.enter",
            "tui.config_toml.loaded",
        ),
        (
            "config_build",
            "tui.config_toml.loaded",
            "tui.config.loaded",
        ),
        (
            "terminal_init",
            "tui.run_ratatui_app.enter",
            "tui.terminal.initialized",
        ),
        (
            "pre_app_server_work",
            "tui.terminal.initialized",
            "tui.start_app_server.begin",
        ),
        (
            "keyboard_enhancement_probe",
            "tui.keyboard_enhancement_probe.begin",
            "tui.keyboard_enhancement_probe.ready",
        ),
        (
            "stdout_color_cache",
            "tui.stdout_color_cache.begin",
            "tui.stdout_color_cache.ready",
        ),
        (
            "default_colors_probe",
            "tui.default_colors_probe.begin",
            "tui.default_colors_probe.ready",
        ),
        (
            "terminal_info_probe",
            "tui.terminal_info.begin",
            "tui.terminal_info.ready",
        ),
        (
            "notification_backend_probe",
            "tui.notification_backend.begin",
            "tui.notification_backend.ready",
        ),
        (
            "start_app_server",
            "tui.start_app_server.begin",
            "tui.start_app_server.ready",
        ),
        (
            "embedded_app_server_start",
            "tui.embedded_app_server.start.begin",
            "tui.embedded_app_server.start.ready",
        ),
        (
            "remote_app_server_connect",
            "tui.remote_app_server.connect.begin",
            "tui.remote_app_server.connect.ready",
        ),
        (
            "post_app_server_work",
            "tui.start_app_server.ready",
            "tui.app_server.ready",
        ),
        (
            "app_server_ready",
            "tui.terminal.initialized",
            "tui.app_server.ready",
        ),
        (
            "bootstrap",
            "tui.app_server.ready",
            "tui.app.bootstrap.complete",
        ),
        (
            "session_initialized",
            "tui.app.bootstrap.complete",
            "tui.session.initialized",
        ),
    ] {
        if let Some(duration) = phase_duration(trace, start_event, end_event) {
            phases.insert(label.to_string(), duration);
        }
    }
    phases
}

fn current_unix_time_ms() -> Result<u128> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("computing benchmark wall-clock start time")?
        .as_millis())
}

fn trace_span_duration(trace: &[StartupTraceEvent]) -> Option<u128> {
    let first = trace.first()?;
    let last = trace.last()?;
    Some(last.unix_time_ms.saturating_sub(first.unix_time_ms))
}

fn submit_prompt_fast(session: &TmuxSession, prompt: &str) -> Result<()> {
    session.send_literal(prompt)?;
    session.send_enter()?;
    Ok(())
}

fn benchmark_env(
    flavor: CliFlavor,
    home: &Path,
    api_key: Option<&str>,
    startup_trace_path: Option<&Path>,
) -> Vec<(String, String)> {
    let mut env = vec![
        ("RUST_LOG".to_string(), "trace".to_string()),
        ("TERM".to_string(), "xterm-256color".to_string()),
        ("COLORTERM".to_string(), "truecolor".to_string()),
    ];
    if let Some(startup_trace_path) = startup_trace_path {
        env.push((
            STARTUP_TRACE_PATH_ENV_VAR.to_string(),
            startup_trace_path.display().to_string(),
        ));
    }
    if let Some(api_key) = api_key {
        env.push(("OPENAI_API_KEY".to_string(), api_key.to_string()));
    }
    match flavor {
        CliFlavor::Codex => {
            env.push(("CODEX_HOME".to_string(), home.display().to_string()));
        }
        CliFlavor::Interpreter => {
            env.push((
                "OPEN_INTERPRETER_HOME".to_string(),
                home.display().to_string(),
            ));
        }
    }
    env
}

fn launch_args(
    _flavor: CliFlavor,
    repo_root: &Path,
    log_dir: &Path,
    profile: Option<BenchmarkProfile>,
) -> Result<Vec<String>> {
    let mut args = base_args(repo_root, log_dir);
    if let Some(profile) = profile {
        args.push("--profile".to_string());
        args.push(profile.label().to_string());
    }
    Ok(args)
}

fn benchmark_client_counts() -> Result<Vec<usize>> {
    parse_client_counts(
        std::env::var("OPEN_INTERPRETER_BENCH_CLIENT_COUNTS")
            .ok()
            .as_deref(),
    )
}

fn parse_client_counts(raw_value: Option<&str>) -> Result<Vec<usize>> {
    let Some(raw_value) = raw_value else {
        return Ok(DEFAULT_BENCH_CLIENT_COUNTS.to_vec());
    };

    let mut counts = Vec::new();
    for raw_count in raw_value
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let count = raw_count
            .parse::<usize>()
            .with_context(|| format!("parsing benchmark client count `{raw_count}`"))?;
        anyhow::ensure!(count > 0, "benchmark client counts must be positive");
        counts.push(count);
    }
    anyhow::ensure!(
        !counts.is_empty(),
        "benchmark client counts must contain at least one positive value"
    );
    Ok(counts)
}

fn benchmark_profiles() -> Result<Vec<BenchmarkProfile>> {
    parse_profiles(
        std::env::var("OPEN_INTERPRETER_BENCH_PROFILES")
            .ok()
            .as_deref(),
    )
}

fn benchmark_flavors() -> Result<Vec<CliFlavor>> {
    parse_flavors(
        std::env::var("OPEN_INTERPRETER_BENCH_FLAVORS")
            .ok()
            .as_deref(),
    )
}

fn parse_flavors(raw_value: Option<&str>) -> Result<Vec<CliFlavor>> {
    let Some(raw_value) = raw_value else {
        return Ok(vec![CliFlavor::Codex, CliFlavor::Interpreter]);
    };

    let mut flavors = Vec::new();
    for raw_flavor in raw_value
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let flavor = match raw_flavor {
            "codex" => CliFlavor::Codex,
            "interpreter" => CliFlavor::Interpreter,
            _ => anyhow::bail!(
                "unknown benchmark flavor `{raw_flavor}`; expected `codex` or `interpreter`"
            ),
        };
        flavors.push(flavor);
    }
    anyhow::ensure!(
        !flavors.is_empty(),
        "benchmark flavors must contain at least one value"
    );
    Ok(flavors)
}

fn parse_profiles(raw_value: Option<&str>) -> Result<Vec<BenchmarkProfile>> {
    let Some(raw_value) = raw_value else {
        return Ok(vec![BenchmarkProfile::Responses, BenchmarkProfile::Chat]);
    };

    let mut profiles = Vec::new();
    for raw_profile in raw_value
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let profile = match raw_profile {
            "responses" => BenchmarkProfile::Responses,
            "chat" => BenchmarkProfile::Chat,
            _ => anyhow::bail!(
                "unknown benchmark profile `{raw_profile}`; expected `responses` or `chat`"
            ),
        };
        profiles.push(profile);
    }
    anyhow::ensure!(
        !profiles.is_empty(),
        "benchmark profiles must contain at least one value"
    );
    Ok(profiles)
}

fn sample_process_memory(pids: &[u32]) -> Result<BTreeMap<String, ProcessMemorySample>> {
    let rss_by_pid = sample_rss_kb(pids)?;

    #[cfg(target_os = "macos")]
    let footprint_by_pid = sample_physical_footprint(pids)?;
    #[cfg(not(target_os = "macos"))]
    let footprint_by_pid: BTreeMap<String, ProcessMemorySample> = BTreeMap::new();

    let mut samples = BTreeMap::new();
    for pid in pids {
        let pid_key = pid.to_string();
        let footprint_sample = footprint_by_pid.get(&pid_key).cloned().unwrap_or_default();
        samples.insert(
            pid_key.clone(),
            ProcessMemorySample {
                rss_kb: rss_by_pid.get(&pid_key).copied().unwrap_or(0),
                phys_footprint_bytes: footprint_sample.phys_footprint_bytes,
                phys_footprint_peak_bytes: footprint_sample.phys_footprint_peak_bytes,
            },
        );
    }

    Ok(samples)
}

#[cfg(target_os = "macos")]
fn sample_physical_footprint(pids: &[u32]) -> Result<BTreeMap<String, ProcessMemorySample>> {
    if pids.is_empty() {
        return Ok(BTreeMap::new());
    }

    let output_file = NamedTempFile::new().context("creating footprint json output file")?;
    let output_path = output_file.path().to_path_buf();

    let mut command = Command::new("footprint");
    command.arg("-f").arg("bytes");
    for pid in pids {
        command.arg("-p").arg(pid.to_string());
    }
    command.arg("-j").arg(&output_path);

    let output = command
        .output()
        .context("running `footprint` for benchmark memory sampling")?;
    anyhow::ensure!(
        output.status.success(),
        "`footprint` failed with status {}",
        output.status
    );

    let footprint: FootprintJson = serde_json::from_str(
        &std::fs::read_to_string(&output_path)
            .with_context(|| format!("reading {}", output_path.display()))?,
    )
    .with_context(|| format!("parsing {}", output_path.display()))?;

    let mut samples = BTreeMap::new();
    for process in footprint.processes {
        samples.insert(
            process.pid.to_string(),
            ProcessMemorySample {
                rss_kb: 0,
                phys_footprint_bytes: process
                    .auxiliary
                    .as_ref()
                    .and_then(|auxiliary| auxiliary.phys_footprint),
                phys_footprint_peak_bytes: process
                    .auxiliary
                    .as_ref()
                    .and_then(|auxiliary| auxiliary.phys_footprint_peak),
            },
        );
    }

    Ok(samples)
}

fn sample_rss_kb(pids: &[u32]) -> Result<BTreeMap<String, u64>> {
    if pids.is_empty() {
        return Ok(BTreeMap::new());
    }

    let pid_list = pids
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let output = Command::new("ps")
        .arg("-o")
        .arg("pid=,rss=")
        .arg("-p")
        .arg(&pid_list)
        .output()
        .with_context(|| format!("running `ps` for PID list `{pid_list}`"))?;
    anyhow::ensure!(
        output.status.success(),
        "`ps` failed while sampling RSS for `{pid_list}`"
    );

    let mut rss_by_pid = BTreeMap::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let mut parts = trimmed.split_whitespace();
        let pid = parts
            .next()
            .context("missing PID column from `ps` output")?
            .parse::<u32>()
            .context("parsing PID column from `ps` output")?;
        let rss_kb = parts
            .next()
            .context("missing RSS column from `ps` output")?
            .parse::<u64>()
            .context("parsing RSS column from `ps` output")?;
        rss_by_pid.insert(pid.to_string(), rss_kb);
    }

    Ok(rss_by_pid)
}

fn sum_rss_kb(samples: &BTreeMap<String, ProcessMemorySample>) -> u64 {
    samples.values().map(|sample| sample.rss_kb).sum()
}

fn sum_phys_footprint_bytes(samples: &BTreeMap<String, ProcessMemorySample>) -> Option<u64> {
    let mut total = 0;
    for sample in samples.values() {
        total += sample.phys_footprint_bytes?;
    }
    Some(total)
}

fn sum_phys_footprint_peak_bytes(samples: &BTreeMap<String, ProcessMemorySample>) -> Option<u64> {
    let mut total = 0;
    for sample in samples.values() {
        total += sample.phys_footprint_peak_bytes?;
    }
    Some(total)
}

fn benchmark_prompts(session_index: usize) -> Vec<(String, String)> {
    let session_tag = format!("SESSION{}_OK", session_index + 1);
    vec![
        (
            format!(
                "Use the shell tool to read the first 120 lines of codex-rs/Cargo.toml, then briefly summarize what this workspace is building. End your reply with exactly {session_tag}_TURN_ONE."
            ),
            format!("{session_tag}_TURN_ONE"),
        ),
        (
            format!(
                "Use the shell tool to read the first 160 lines of codex-rs/core/src/lib.rs, then briefly summarize what the core crate exposes. End your reply with exactly {session_tag}_TURN_TWO."
            ),
            format!("{session_tag}_TURN_TWO"),
        ),
        (
            format!(
                "Use the shell tool to read the first 200 lines of codex-rs/app-server/src/lib.rs, then briefly summarize what the app server owns. End your reply with exactly {session_tag}_TURN_THREE."
            ),
            format!("{session_tag}_TURN_THREE"),
        ),
    ]
}

fn base_args(repo_root: &Path, log_dir: &Path) -> Vec<String> {
    vec![
        "--no-alt-screen".to_string(),
        "-C".to_string(),
        repo_root.display().to_string(),
        "-c".to_string(),
        "analytics.enabled=false".to_string(),
        "-c".to_string(),
        format!("log_dir=\"{}\"", log_dir.display()),
    ]
}

fn write_mock_config(home: &Path, repo_root: &Path, responses_base_url: &str) -> Result<()> {
    let config_contents = format!(
        r#"
model = "{TEST_MODEL}"
model_provider = "mock_responses"
approval_policy = "never"
sandbox_mode = "read-only"

[features]
apps = false
plugins = false

[projects."{}"]
trust_level = "trusted"

[model_providers.mock_responses]
name = "Mock Responses"
base_url = "{responses_base_url}/v1"
env_key = "PATH"
wire_api = "responses"
requires_openai_auth = false
request_max_retries = 0
stream_max_retries = 0
supports_websockets = false
"#,
        repo_root.display()
    );
    std::fs::write(home.join("config.toml"), config_contents)
        .with_context(|| format!("failed to write {}", home.join("config.toml").display()))?;
    Ok(())
}

fn write_live_config(home: &Path, repo_root: &Path) -> Result<()> {
    let config_contents = format!(
        r#"
model = "{TEST_MODEL}"
model_provider = "openai_responses_api_key"
approval_policy = "never"
sandbox_mode = "read-only"

[features]
apps = false
plugins = false

[profiles.responses]
model = "{TEST_MODEL}"
model_provider = "openai_responses_api_key"

[profiles.chat]
model = "{TEST_MODEL}"
model_provider = "openai_chat_completions"

[projects."{}"]
trust_level = "trusted"

[model_providers.openai_responses_api_key]
name = "OpenAI Responses API Key"
base_url = "https://api.openai.com/v1"
env_key = "OPENAI_API_KEY"
wire_api = "responses"
requires_openai_auth = false
request_max_retries = 0
stream_max_retries = 0
supports_websockets = false

[model_providers.openai_chat_completions]
name = "OpenAI Chat Completions"
base_url = "https://api.openai.com/v1"
env_key = "OPENAI_API_KEY"
wire_api = "chat"
requires_openai_auth = false
request_max_retries = 0
stream_max_retries = 0
supports_websockets = false
"#,
        repo_root.display()
    );
    std::fs::write(home.join("config.toml"), config_contents)
        .with_context(|| format!("failed to write {}", home.join("config.toml").display()))?;
    Ok(())
}

fn read_daemon_lockfile(home: &Path) -> Result<DaemonLockfile> {
    let lockfile_path = home.join("tmp").join("interpreter").join("app-server.json");
    let content = std::fs::read_to_string(&lockfile_path)
        .with_context(|| format!("failed to read {}", lockfile_path.display()))?;
    serde_json::from_str(&content)
        .with_context(|| format!("failed to parse {}", lockfile_path.display()))
}

async fn wait_for_daemon_exit_if_present(home: &Path) -> Result<()> {
    let lockfile_path = home.join("tmp").join("interpreter").join("app-server.json");
    if !lockfile_path.exists() {
        return Ok(());
    }

    let daemon = read_daemon_lockfile(home)?;
    wait_for_pid_exit(daemon.pid, Duration::from_secs(15))
        .await
        .with_context(|| format!("waiting for daemon PID {} to exit", daemon.pid))
}

async fn wait_for_pid_exit(pid: u32, timeout_duration: Duration) -> Result<()> {
    timeout(timeout_duration, async {
        loop {
            if !pid_is_alive(pid)? {
                return Ok(());
            }
            sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .with_context(|| format!("timed out waiting for PID {pid} to exit"))?
}

fn pid_is_alive(pid: u32) -> Result<bool> {
    let status = Command::new("ps")
        .arg("-p")
        .arg(pid.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .with_context(|| format!("running `ps` to probe PID {pid}"))?;
    Ok(status.success())
}

fn normalize_screen(screen: &str) -> String {
    let mut lines = screen
        .lines()
        .map(|line| line.trim_end().to_string())
        .collect::<Vec<_>>();
    while matches!(lines.last(), Some(last) if last.is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}

#[test]
fn parse_client_counts_defaults_to_standard_sweep() -> Result<()> {
    assert_eq!(
        parse_client_counts(None)?,
        DEFAULT_BENCH_CLIENT_COUNTS.to_vec()
    );
    Ok(())
}

#[test]
fn parse_client_counts_rejects_zero() {
    let error = parse_client_counts(Some("0")).unwrap_err();
    assert_eq!(
        error.to_string(),
        "benchmark client counts must be positive"
    );
}

#[test]
fn parse_profiles_defaults_to_responses_and_chat() -> Result<()> {
    assert_eq!(
        parse_profiles(None)?,
        vec![BenchmarkProfile::Responses, BenchmarkProfile::Chat]
    );
    Ok(())
}

#[test]
fn parse_profiles_rejects_unknown_values() {
    let error = parse_profiles(Some("responses,banana")).unwrap_err();
    assert_eq!(
        error.to_string(),
        "unknown benchmark profile `banana`; expected `responses` or `chat`"
    );
}

#[test]
fn parse_flavors_defaults_to_codex_and_interpreter() -> Result<()> {
    assert_eq!(
        parse_flavors(None)?,
        vec![CliFlavor::Codex, CliFlavor::Interpreter]
    );
    Ok(())
}

#[test]
fn parse_flavors_rejects_unknown_values() {
    let error = parse_flavors(Some("interpreter,banana")).unwrap_err();
    assert_eq!(
        error.to_string(),
        "unknown benchmark flavor `banana`; expected `codex` or `interpreter`"
    );
}
