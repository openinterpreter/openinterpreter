#![deny(clippy::print_stdout, clippy::print_stderr)]

use codex_arg0::Arg0DispatchPaths;
use codex_core::config::Config;
use codex_core::config::ConfigBuilder;
use codex_core::config_loader::CloudRequirementsLoader;
use codex_core::config_loader::ConfigLayerStackOrdering;
use codex_core::config_loader::LoaderOverrides;
use codex_features::Feature;
use codex_login::AuthManager;
use codex_utils_cli::CliConfigOverrides;
use std::collections::HashMap;
use std::collections::HashSet;
use std::ffi::OsStr;
use std::io::ErrorKind;
use std::io::Result as IoResult;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::RwLock;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use crate::cloud_requirements_loader::build_cloud_requirements_loader;
use crate::message_processor::MessageProcessor;
use crate::message_processor::MessageProcessorArgs;
use crate::outgoing_message::ConnectionId;
use crate::outgoing_message::OutgoingEnvelope;
use crate::outgoing_message::OutgoingMessageSender;
use crate::outgoing_message::QueuedOutgoingMessage;
use crate::startup_trace::record_startup_trace_event;
use crate::transport::CHANNEL_CAPACITY;
use crate::transport::ConnectionState;
use crate::transport::OutboundConnectionState;
use crate::transport::TransportEvent;
use crate::transport::auth::policy_from_settings;
use crate::transport::route_outgoing_envelope;
use crate::transport::start_remote_control;
use crate::transport::start_stdio_connection;
use crate::transport::start_websocket_acceptor;
use codex_analytics::AppServerRpcTransport;
use codex_app_server_protocol::ConfigLayerSource;
use codex_app_server_protocol::ConfigWarningNotification;
use codex_app_server_protocol::JSONRPCMessage;
use codex_app_server_protocol::TextPosition as AppTextPosition;
use codex_app_server_protocol::TextRange as AppTextRange;
use codex_core::ExecPolicyError;
use codex_core::check_execpolicy_for_warnings;
use codex_core::config_loader::ConfigLoadError;
use codex_core::config_loader::TextRange as CoreTextRange;
use codex_exec_server::EnvironmentManager;
use codex_exec_server::ExecServerRuntimePaths;
use codex_feedback::CodexFeedback;
use codex_protocol::protocol::SessionSource;
use codex_state::log_db;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use toml::Value as TomlValue;
use tracing::Level;
use tracing::error;
use tracing::info;
use tracing::warn;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::Layer;
use tracing_subscriber::filter::Targets;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::registry::Registry;
use tracing_subscriber::util::SubscriberInitExt;

mod app_server_tracing;
mod bespoke_event_handling;
mod cli_startup;
mod cloud_requirements_loader;
mod codex_message_processor;
mod command_exec;
mod config_api;
mod dynamic_tools;
mod error_code;
mod external_agent_config_api;
mod filters;
mod fs_api;
mod fs_watch;
mod fuzzy_file_search;
pub mod in_process;
mod message_processor;
mod models;
mod outgoing_message;
mod server_request_error;
mod startup_trace;
mod thread_state;
mod thread_status;
mod transport;

pub use crate::cli_startup::run_main_from_cli_args;
pub use crate::error_code::INPUT_TOO_LARGE_ERROR_CODE;
pub use crate::error_code::INVALID_PARAMS_ERROR_CODE;
pub use crate::transport::AppServerTransport;
pub use crate::transport::auth::AppServerWebsocketAuthArgs;
pub use crate::transport::auth::AppServerWebsocketAuthSettings;
pub use crate::transport::auth::WebsocketAuthCliMode;

const LOG_FORMAT_ENV_VAR: &str = "LOG_FORMAT";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LogFormat {
    Default,
    Json,
}

type StderrLogLayer = Box<dyn Layer<Registry> + Send + Sync + 'static>;

/// Control-plane messages from the processor/transport side to the outbound router task.
///
/// `run_main_with_transport` now uses two loops/tasks:
/// - processor loop: handles incoming JSON-RPC and request dispatch
/// - outbound loop: performs potentially slow writes to per-connection writers
///
/// `OutboundControlEvent` keeps those loops coordinated without sharing mutable
/// connection state directly. In particular, the outbound loop needs to know
/// when a connection opens/closes so it can route messages correctly.
enum OutboundControlEvent {
    /// Register a new writer for an opened connection.
    Opened {
        connection_id: ConnectionId,
        writer: mpsc::Sender<QueuedOutgoingMessage>,
        disconnect_sender: Option<CancellationToken>,
        initialized: Arc<AtomicBool>,
        experimental_api_enabled: Arc<AtomicBool>,
        opted_out_notification_methods: Arc<RwLock<HashSet<String>>>,
    },
    /// Remove state for a closed/disconnected connection.
    Closed { connection_id: ConnectionId },
    /// Disconnect all connection-oriented clients during graceful restart.
    DisconnectAll,
}

#[derive(Default)]
struct ShutdownState {
    requested: bool,
    forced: bool,
    last_logged_running_turn_count: Option<usize>,
}

enum ShutdownAction {
    Noop,
    Finish,
}

async fn shutdown_signal() -> IoResult<()> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::SignalKind;
        use tokio::signal::unix::signal;

        let mut term = signal(SignalKind::terminate())?;
        tokio::select! {
            ctrl_c_result = tokio::signal::ctrl_c() => ctrl_c_result,
            _ = term.recv() => Ok(()),
        }
    }

    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await
    }
}

impl ShutdownState {
    fn requested(&self) -> bool {
        self.requested
    }

    fn forced(&self) -> bool {
        self.forced
    }

    fn on_signal(&mut self, connection_count: usize, running_turn_count: usize) {
        if self.requested {
            self.forced = true;
            return;
        }

        self.requested = true;
        self.last_logged_running_turn_count = None;
        info!(
            "received shutdown signal; entering graceful restart drain (connections={}, runningAssistantTurns={}, requests still accepted until no assistant turns are running)",
            connection_count, running_turn_count,
        );
    }

    fn update(&mut self, running_turn_count: usize, connection_count: usize) -> ShutdownAction {
        if !self.requested {
            return ShutdownAction::Noop;
        }

        if self.forced || running_turn_count == 0 {
            if self.forced {
                info!(
                    "received second shutdown signal; forcing restart with {running_turn_count} running assistant turn(s) and {connection_count} connection(s)"
                );
            } else {
                info!(
                    "shutdown signal restart: no assistant turns running; stopping acceptor and disconnecting {connection_count} connection(s)"
                );
            }
            return ShutdownAction::Finish;
        }

        if self.last_logged_running_turn_count != Some(running_turn_count) {
            info!(
                "shutdown signal restart: waiting for {running_turn_count} running assistant turn(s) to finish"
            );
            self.last_logged_running_turn_count = Some(running_turn_count);
        }

        ShutdownAction::Noop
    }
}

fn config_warning_from_error(
    summary: impl Into<String>,
    err: &std::io::Error,
) -> ConfigWarningNotification {
    let (path, range) = match config_error_location(err) {
        Some((path, range)) => (Some(path), Some(range)),
        None => (None, None),
    };
    ConfigWarningNotification {
        summary: summary.into(),
        details: Some(err.to_string()),
        path,
        range,
    }
}

fn config_error_location(err: &std::io::Error) -> Option<(String, AppTextRange)> {
    err.get_ref()
        .and_then(|err| err.downcast_ref::<ConfigLoadError>())
        .map(|err| {
            let config_error = err.config_error();
            (
                config_error.path.to_string_lossy().to_string(),
                app_text_range(&config_error.range),
            )
        })
}

fn exec_policy_warning_location(err: &ExecPolicyError) -> (Option<String>, Option<AppTextRange>) {
    match err {
        ExecPolicyError::ParsePolicy { path, source } => {
            if let Some(location) = source.location() {
                let range = AppTextRange {
                    start: AppTextPosition {
                        line: location.range.start.line,
                        column: location.range.start.column,
                    },
                    end: AppTextPosition {
                        line: location.range.end.line,
                        column: location.range.end.column,
                    },
                };
                return (Some(location.path), Some(range));
            }
            (Some(path.clone()), None)
        }
        _ => (None, None),
    }
}

fn app_text_range(range: &CoreTextRange) -> AppTextRange {
    AppTextRange {
        start: AppTextPosition {
            line: range.start.line,
            column: range.start.column,
        },
        end: AppTextPosition {
            line: range.end.line,
            column: range.end.column,
        },
    }
}

fn project_config_warning(config: &Config) -> Option<ConfigWarningNotification> {
    let disabled_folders = disabled_project_config_warning_entries(config);

    if disabled_folders.is_empty() {
        return None;
    }

    let mut message = concat!(
        "Project-local config, hooks, and exec policies are disabled for the following projects ",
        "until they are trusted, but skills still load.\n",
    )
    .to_string();
    for (index, (folder, reason)) in disabled_folders.iter().enumerate() {
        let display_index = index + 1;
        message.push_str(&format!("    {display_index}. {folder}\n"));
        message.push_str(&format!("       {reason}\n"));
    }

    Some(ConfigWarningNotification {
        summary: message,
        details: None,
        path: None,
        range: None,
    })
}

fn disabled_project_config_warning_entries(config: &Config) -> Vec<(String, String)> {
    disabled_project_config_warning_entries_with_home(config, home_dir().as_deref())
}

fn disabled_project_config_warning_entries_with_home(
    config: &Config,
    home_dir_override: Option<&Path>,
) -> Vec<(String, String)> {
    let legacy_codex_home = home_dir_override.map(|home| home.join(".codex"));
    let mut disabled_folders = Vec::new();

    for layer in config.config_layer_stack.get_layers(
        ConfigLayerStackOrdering::LowestPrecedenceFirst,
        /*include_disabled*/ true,
    ) {
        let ConfigLayerSource::Project { dot_codex_folder } = &layer.name else {
            continue;
        };
        if layer.disabled_reason.is_none() {
            continue;
        }
        if legacy_codex_home.as_deref() == Some(dot_codex_folder.as_path())
            && config.codex_home.as_path() != dot_codex_folder.as_path()
        {
            continue;
        }
        disabled_folders.push((
            project_warning_display_path(dot_codex_folder.as_path()),
            layer
                .disabled_reason
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| "config.toml is disabled.".to_string()),
        ));
    }

    disabled_folders
}

fn project_warning_display_path(path: &Path) -> String {
    for ancestor in path.ancestors() {
        if ancestor.file_name() == Some(OsStr::new(".codex"))
            && let Some(parent) = ancestor.parent()
        {
            return parent.display().to_string();
        }
    }

    path.display().to_string()
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        })
}

fn runtime_auth_manager(config: &Config, enable_codex_api_key_env: bool) -> Arc<AuthManager> {
    AuthManager::shared_from_config(config, enable_codex_api_key_env)
}

async fn start_runtime_remote_control(
    config: &Config,
    state_db: Option<Arc<codex_state::StateRuntime>>,
    auth_manager: Arc<AuthManager>,
    transport_event_tx: mpsc::Sender<TransportEvent>,
    shutdown_token: CancellationToken,
    app_server_client_name_rx: Option<oneshot::Receiver<String>>,
) -> IoResult<(JoinHandle<()>, crate::transport::RemoteControlHandle)> {
    start_remote_control(
        config.chatgpt_base_url.clone(),
        state_db,
        auth_manager,
        transport_event_tx,
        shutdown_token,
        app_server_client_name_rx,
        config.features.enabled(Feature::RemoteControl),
    )
    .await
}

impl LogFormat {
    fn from_env_value(value: Option<&str>) -> Self {
        match value.map(str::trim).map(str::to_ascii_lowercase) {
            Some(value) if value == "json" => Self::Json,
            _ => Self::Default,
        }
    }
}

fn log_format_from_env() -> LogFormat {
    let value = std::env::var(LOG_FORMAT_ENV_VAR).ok();
    LogFormat::from_env_value(value.as_deref())
}

fn should_arm_idle_shutdown(
    has_seen_connection: bool,
    connection_count: usize,
    running_turn_count: usize,
) -> bool {
    has_seen_connection && connection_count == 0 && running_turn_count == 0
}

pub async fn run_main(
    arg0_paths: Arg0DispatchPaths,
    cli_config_overrides: CliConfigOverrides,
    loader_overrides: LoaderOverrides,
    default_analytics_enabled: bool,
) -> IoResult<()> {
    run_main_with_transport(
        arg0_paths,
        cli_config_overrides,
        loader_overrides,
        RunMainOptions {
            default_analytics_enabled,
            transport: AppServerTransport::Stdio,
            session_source: SessionSource::VSCode,
            ..RunMainOptions::default()
        },
        AppServerWebsocketAuthSettings::default(),
    )
    .await
}

#[derive(Debug)]
pub struct RunMainOptions {
    pub default_analytics_enabled: bool,
    pub enable_codex_api_key_env: bool,
    pub shutdown_idle_timeout: Option<Duration>,
    pub transport: AppServerTransport,
    pub session_source: SessionSource,
}

impl Default for RunMainOptions {
    fn default() -> Self {
        Self {
            default_analytics_enabled: false,
            enable_codex_api_key_env: false,
            shutdown_idle_timeout: None,
            transport: AppServerTransport::Stdio,
            session_source: SessionSource::VSCode,
        }
    }
}

pub async fn run_main_with_transport(
    arg0_paths: Arg0DispatchPaths,
    cli_config_overrides: CliConfigOverrides,
    loader_overrides: LoaderOverrides,
    options: RunMainOptions,
    auth: AppServerWebsocketAuthSettings,
) -> IoResult<()> {
    record_startup_trace_event("app_server.run_main.enter");
    let RunMainOptions {
        default_analytics_enabled,
        enable_codex_api_key_env,
        shutdown_idle_timeout,
        transport,
        session_source,
    } = options;
    let environment_manager = Arc::new(EnvironmentManager::from_env_with_runtime_paths(Some(
        ExecServerRuntimePaths::from_optional_paths(
            arg0_paths.codex_self_exe.clone(),
            arg0_paths.codex_linux_sandbox_exe.clone(),
        )?,
    )));
    let (transport_event_tx, mut transport_event_rx) =
        mpsc::channel::<TransportEvent>(CHANNEL_CAPACITY);
    let (outgoing_tx, mut outgoing_rx) = mpsc::channel::<OutgoingEnvelope>(CHANNEL_CAPACITY);
    let (outbound_control_tx, mut outbound_control_rx) =
        mpsc::channel::<OutboundControlEvent>(CHANNEL_CAPACITY);

    // Parse CLI overrides once and derive the base Config eagerly so later
    // components do not need to work with raw TOML values.
    record_startup_trace_event("app_server.cli_overrides.parse.begin");
    let cli_kv_overrides = cli_config_overrides.parse_overrides().map_err(|e| {
        std::io::Error::new(
            ErrorKind::InvalidInput,
            format!("error parsing -c overrides: {e}"),
        )
    })?;
    record_startup_trace_event("app_server.cli_overrides.parse.ready");
    let transport_shutdown_token = CancellationToken::new();
    let mut transport_accept_handles = Vec::<JoinHandle<()>>::new();

    let single_client_mode = matches!(&transport, AppServerTransport::Stdio);
    let shutdown_when_no_connections = single_client_mode;
    let shutdown_idle_timeout = if single_client_mode {
        None
    } else {
        shutdown_idle_timeout
    };
    let graceful_signal_restart_enabled = !single_client_mode;
    let (mut initialize_client_name_tx, initialize_client_name_rx) = match transport {
        AppServerTransport::Stdio => {
            let (tx, rx) = oneshot::channel::<String>();
            (Some(tx), Some(rx))
        }
        AppServerTransport::WebSocket { .. } | AppServerTransport::Off => (None, None),
    };

    if let AppServerTransport::WebSocket { bind_address } = transport {
        record_startup_trace_event("app_server.websocket_acceptor.start");
        let accept_handle = start_websocket_acceptor(
            bind_address,
            transport_event_tx.clone(),
            transport_shutdown_token.clone(),
            policy_from_settings(&auth)?,
        )
        .await?;
        transport_accept_handles.push(accept_handle);
        record_startup_trace_event("app_server.websocket_acceptor.ready");
    }

    record_startup_trace_event("app_server.config_preload.begin");
    let cloud_requirements = match ConfigBuilder::default()
        .cli_overrides(cli_kv_overrides.clone())
        .loader_overrides(loader_overrides.clone())
        .build()
        .await
    {
        Ok(config) => {
            let effective_toml = config.config_layer_stack.effective_config();
            match effective_toml.try_into() {
                Ok(config_toml) => {
                    if let Err(err) = codex_core::personality_migration::maybe_migrate_personality(
                        &config.codex_home,
                        &config_toml,
                    )
                    .await
                    {
                        warn!(error = %err, "Failed to run personality migration");
                    }
                }
                Err(err) => {
                    warn!(error = %err, "Failed to deserialize config for personality migration");
                }
            }

            let auth_manager = AuthManager::shared(
                config.codex_home.clone().to_path_buf(),
                enable_codex_api_key_env,
                config.cli_auth_credentials_store_mode,
            );
            build_cloud_requirements_loader(
                auth_manager,
                config.chatgpt_base_url,
                config.codex_home.to_path_buf(),
            )
        }
        Err(err) => {
            warn!(error = %err, "Failed to preload config for cloud requirements");
            // TODO(gt): Make cloud requirements preload failures blocking once we can fail-closed.
            CloudRequirementsLoader::default()
        }
    };
    record_startup_trace_event("app_server.config_preload.ready");
    let loader_overrides_for_config_api = loader_overrides.clone();
    let mut config_warnings = Vec::new();
    record_startup_trace_event("app_server.config.begin");
    let config = match ConfigBuilder::default()
        .cli_overrides(cli_kv_overrides.clone())
        .loader_overrides(loader_overrides)
        .cloud_requirements(cloud_requirements.clone())
        .build()
        .await
    {
        Ok(config) => config,
        Err(err) => {
            let message = config_warning_from_error("Invalid configuration; using defaults.", &err);
            config_warnings.push(message);
            Config::load_default_with_cli_overrides(cli_kv_overrides.clone())
                .await
                .map_err(|e| {
                    std::io::Error::new(
                        ErrorKind::InvalidData,
                        format!("error loading default config after config error: {e}"),
                    )
                })?
        }
    };
    record_startup_trace_event("app_server.config.ready");

    if let Ok(Some(err)) = check_execpolicy_for_warnings(&config.config_layer_stack).await {
        let (path, range) = exec_policy_warning_location(&err);
        let message = ConfigWarningNotification {
            summary: "Error parsing rules; custom rules not applied.".to_string(),
            details: Some(err.to_string()),
            path,
            range,
        };
        config_warnings.push(message);
    }

    if let Some(warning) = project_config_warning(&config) {
        config_warnings.push(warning);
    }
    for warning in &config.startup_warnings {
        config_warnings.push(ConfigWarningNotification {
            summary: warning.clone(),
            details: None,
            path: None,
            range: None,
        });
    }
    if let Some(warning) =
        codex_core::config::system_bwrap_warning(config.permissions.sandbox_policy.get())
    {
        config_warnings.push(ConfigWarningNotification {
            summary: warning,
            details: None,
            path: None,
            range: None,
        });
    }

    let feedback = CodexFeedback::new();

    record_startup_trace_event("app_server.observability.begin");
    let otel = codex_core::otel_init::build_provider(
        &config,
        env!("CARGO_PKG_VERSION"),
        Some("codex-app-server"),
        default_analytics_enabled,
    )
    .map_err(|e| {
        std::io::Error::new(
            ErrorKind::InvalidData,
            format!("error loading otel config: {e}"),
        )
    })?;

    // Install a simple subscriber so `tracing` output is visible. Users can
    // control the log level with `RUST_LOG` and switch to JSON logs with
    // `LOG_FORMAT=json`.
    let stderr_fmt: StderrLogLayer = match log_format_from_env() {
        LogFormat::Json => tracing_subscriber::fmt::layer()
            .json()
            .with_writer(std::io::stderr)
            .with_span_events(tracing_subscriber::fmt::format::FmtSpan::FULL)
            .with_filter(EnvFilter::from_default_env())
            .boxed(),
        LogFormat::Default => tracing_subscriber::fmt::layer()
            .with_writer(std::io::stderr)
            .with_span_events(tracing_subscriber::fmt::format::FmtSpan::FULL)
            .with_filter(EnvFilter::from_default_env())
            .boxed(),
    };

    let feedback_layer = feedback.logger_layer();
    let feedback_metadata_layer = feedback.metadata_layer();
    let state_db = codex_state::StateRuntime::init(
        config.sqlite_home.clone(),
        config.model_provider_id.clone(),
    )
    .await
    .ok();
    let log_db = state_db.clone().map(log_db::start);
    let log_db_layer = log_db
        .clone()
        .map(|layer| layer.with_filter(Targets::new().with_default(Level::TRACE)));
    let otel_logger_layer = otel.as_ref().and_then(|o| o.logger_layer());
    let otel_tracing_layer = otel.as_ref().and_then(|o| o.tracing_layer());
    let _ = tracing_subscriber::registry()
        .with(stderr_fmt)
        .with(feedback_layer)
        .with(feedback_metadata_layer)
        .with(log_db_layer)
        .with(otel_logger_layer)
        .with(otel_tracing_layer)
        .try_init();
    for warning in &config_warnings {
        match &warning.details {
            Some(details) => error!("{} {}", warning.summary, details),
            None => error!("{}", warning.summary),
        }
    }
    record_startup_trace_event("app_server.observability.ready");

    let auth_manager = runtime_auth_manager(&config, enable_codex_api_key_env);
    let (remote_control_task, remote_control_handle) = start_runtime_remote_control(
        &config,
        state_db.clone(),
        auth_manager.clone(),
        transport_event_tx.clone(),
        transport_shutdown_token.clone(),
        initialize_client_name_rx,
    )
    .await?;
    transport_accept_handles.push(remote_control_task);

    match transport {
        AppServerTransport::Stdio => {
            record_startup_trace_event("app_server.stdio_connection.start");
            let initialize_client_name_tx = initialize_client_name_tx.take().ok_or_else(|| {
                std::io::Error::other("stdio transport missing client-name sender")
            })?;
            start_stdio_connection(
                transport_event_tx.clone(),
                &mut transport_accept_handles,
                initialize_client_name_tx,
            )
            .await?;
            record_startup_trace_event("app_server.stdio_connection.ready");
        }
        AppServerTransport::WebSocket { .. } => {}
        AppServerTransport::Off => {}
    }

    record_startup_trace_event("app_server.outbound_router.start");
    let outbound_handle = tokio::spawn(async move {
        let mut outbound_connections = HashMap::<ConnectionId, OutboundConnectionState>::new();
        loop {
            tokio::select! {
                    biased;
                    event = outbound_control_rx.recv() => {
                        let Some(event) = event else {
                            break;
                        };
                        match event {
                            OutboundControlEvent::Opened {
                                connection_id,
                                writer,
                                disconnect_sender,
                                initialized,
                                experimental_api_enabled,
                                opted_out_notification_methods,
                            } => {
                                outbound_connections.insert(
                                    connection_id,
                                    OutboundConnectionState::new(
                                        writer,
                                        initialized,
                                        experimental_api_enabled,
                                        opted_out_notification_methods,
                                        disconnect_sender,
                                    ),
                                );
                            }
                            OutboundControlEvent::Closed { connection_id } => {
                                outbound_connections.remove(&connection_id);
                            }
                            OutboundControlEvent::DisconnectAll => {
                                info!(
                                    "disconnecting {} outbound websocket connection(s) for graceful restart",
                                    outbound_connections.len()
                                );
                                for connection_state in outbound_connections.values() {
                                    connection_state.request_disconnect();
                                }
                                outbound_connections.clear();
                            }
                        }
                    }
                    envelope = outgoing_rx.recv() => {
                    let Some(envelope) = envelope else {
                        break;
                    };
                    route_outgoing_envelope(&mut outbound_connections, envelope).await;
                }
            }
        }
        info!("outbound router task exited (channel closed)");
    });
    record_startup_trace_event("app_server.outbound_router.ready");

    record_startup_trace_event("app_server.processor.start");
    let processor_handle = tokio::spawn({
        let outgoing_message_sender = Arc::new(OutgoingMessageSender::new(outgoing_tx));
        let outbound_control_tx = outbound_control_tx;
        let cli_overrides: Vec<(String, TomlValue)> = cli_kv_overrides.clone();
        let loader_overrides = loader_overrides_for_config_api;
        let processor = Arc::new(MessageProcessor::new(MessageProcessorArgs {
            outgoing: outgoing_message_sender,
            arg0_paths,
            config: Arc::new(config),
            environment_manager,
            cli_overrides,
            loader_overrides,
            cloud_requirements: cloud_requirements.clone(),
            feedback: feedback.clone(),
            log_db,
            config_warnings,
            session_source,
            auth_manager,
            rpc_transport: analytics_rpc_transport(transport),
            remote_control_handle: Some(remote_control_handle),
        }));
        let mut thread_created_rx = processor.thread_created_receiver();
        let mut running_turn_count_rx = processor.subscribe_running_assistant_turn_count();
        let mut connections = HashMap::<ConnectionId, ConnectionState>::new();
        let transport_shutdown_token = transport_shutdown_token.clone();
        async move {
            let mut listen_for_threads = true;
            let mut shutdown_state = ShutdownState::default();
            let mut has_seen_connection = false;
            let mut shutdown_idle_deadline =
                shutdown_idle_timeout.map(|timeout| tokio::time::Instant::now() + timeout);
            loop {
                let running_turn_count = {
                    let running_turn_count = running_turn_count_rx.borrow();
                    *running_turn_count
                };
                if should_arm_idle_shutdown(
                    has_seen_connection,
                    connections.len(),
                    running_turn_count,
                ) {
                    if shutdown_idle_deadline.is_none() {
                        shutdown_idle_deadline = shutdown_idle_timeout
                            .map(|timeout| tokio::time::Instant::now() + timeout);
                    }
                } else if has_seen_connection {
                    shutdown_idle_deadline = None;
                }
                if matches!(
                    shutdown_state.update(running_turn_count, connections.len()),
                    ShutdownAction::Finish
                ) {
                    transport_shutdown_token.cancel();
                    let _ = outbound_control_tx
                        .send(OutboundControlEvent::DisconnectAll)
                        .await;
                    break;
                }

                tokio::select! {
                    shutdown_signal_result = shutdown_signal(), if graceful_signal_restart_enabled && !shutdown_state.forced() => {
                        if let Err(err) = shutdown_signal_result {
                            warn!("failed to listen for shutdown signal during graceful restart drain: {err}");
                        }
                        let running_turn_count = *running_turn_count_rx.borrow();
                        shutdown_state.on_signal(connections.len(), running_turn_count);
                    }
                    changed = running_turn_count_rx.changed(), if graceful_signal_restart_enabled && (shutdown_state.requested() || shutdown_idle_timeout.is_some()) => {
                        if changed.is_err() {
                            warn!("running-turn watcher closed during graceful restart drain");
                        }
                    }
                    _ = async {
                        if let Some(deadline) = shutdown_idle_deadline {
                            tokio::time::sleep_until(deadline).await;
                        } else {
                            std::future::pending::<()>().await;
                        }
                    } => {
                        if !has_seen_connection
                            && connections.is_empty()
                            && *running_turn_count_rx.borrow() == 0
                        {
                            info!("shutting down websocket app-server after startup grace timeout without any connected clients");
                            break;
                        }
                        if should_arm_idle_shutdown(
                            has_seen_connection,
                            connections.len(),
                            *running_turn_count_rx.borrow(),
                        ) {
                            info!("shutting down websocket app-server after idle timeout with no active connections or running assistant turns");
                            break;
                        }
                        shutdown_idle_deadline = None;
                    }
                    event = transport_event_rx.recv() => {
                        let Some(event) = event else {
                            break;
                        };
                        match event {
                            TransportEvent::ConnectionOpened {
                                connection_id,
                                writer,
                                disconnect_sender,
                            } => {
                                has_seen_connection = true;
                                shutdown_idle_deadline = None;
                                let outbound_initialized = Arc::new(AtomicBool::new(false));
                                let outbound_experimental_api_enabled =
                                    Arc::new(AtomicBool::new(false));
                                let outbound_opted_out_notification_methods =
                                    Arc::new(RwLock::new(HashSet::new()));
                                if outbound_control_tx
                                    .send(OutboundControlEvent::Opened {
                                        connection_id,
                                        writer,
                                        disconnect_sender,
                                        initialized: Arc::clone(&outbound_initialized),
                                        experimental_api_enabled: Arc::clone(
                                            &outbound_experimental_api_enabled,
                                        ),
                                        opted_out_notification_methods: Arc::clone(
                                            &outbound_opted_out_notification_methods,
                                        ),
                                    })
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                                connections.insert(
                                    connection_id,
                                    ConnectionState::new(
                                        outbound_initialized,
                                        outbound_experimental_api_enabled,
                                        outbound_opted_out_notification_methods,
                                    ),
                                );
                            }
                            TransportEvent::ConnectionClosed { connection_id } => {
                                if connections.remove(&connection_id).is_none() {
                                    continue;
                                }
                                if outbound_control_tx
                                    .send(OutboundControlEvent::Closed { connection_id })
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                                processor.connection_closed(connection_id).await;
                                if shutdown_when_no_connections && connections.is_empty() {
                                    break;
                                }
                            }
                            TransportEvent::IncomingMessage { connection_id, message } => {
                                match message {
                                    JSONRPCMessage::Request(request) => {
                                        let Some(connection_state) = connections.get_mut(&connection_id) else {
                                            warn!("dropping request from unknown connection: {connection_id:?}");
                                            continue;
                                        };
                                        let was_initialized =
                                            connection_state.session.initialized();
                                        processor
                                            .process_request(
                                                connection_id,
                                                request,
                                                transport,
                                                Arc::clone(&connection_state.session),
                                            )
                                            .await;
                                        let opted_out_notification_methods_snapshot = connection_state
                                            .session
                                            .opted_out_notification_methods();
                                        let experimental_api_enabled =
                                            connection_state.session.experimental_api_enabled();
                                        let is_initialized = connection_state.session.initialized();
                                        if let Ok(mut opted_out_notification_methods) = connection_state
                                            .outbound_opted_out_notification_methods
                                            .write()
                                        {
                                            *opted_out_notification_methods =
                                                opted_out_notification_methods_snapshot;
                                        } else {
                                            warn!(
                                                "failed to update outbound opted-out notifications"
                                            );
                                        }
                                        connection_state
                                            .outbound_experimental_api_enabled
                                            .store(
                                                experimental_api_enabled,
                                                std::sync::atomic::Ordering::Release,
                                            );
                                        if !was_initialized && is_initialized {
                                            processor
                                                .send_initialize_notifications_to_connection(
                                                    connection_id,
                                                )
                                                .await;
                                            processor.connection_initialized(connection_id).await;
                                            connection_state
                                                .outbound_initialized
                                                .store(true, std::sync::atomic::Ordering::Release);
                                        }
                                    }
                                    JSONRPCMessage::Response(response) => {
                                        if !connections.contains_key(&connection_id) {
                                            warn!("dropping response from unknown connection: {connection_id:?}");
                                            continue;
                                        }
                                        processor.process_response(response).await;
                                    }
                                    JSONRPCMessage::Notification(notification) => {
                                        if !connections.contains_key(&connection_id) {
                                            warn!("dropping notification from unknown connection: {connection_id:?}");
                                            continue;
                                        }
                                        processor.process_notification(notification).await;
                                    }
                                    JSONRPCMessage::Error(err) => {
                                        if !connections.contains_key(&connection_id) {
                                            warn!("dropping error from unknown connection: {connection_id:?}");
                                            continue;
                                        }
                                        processor.process_error(err).await;
                                    }
                                }
                            }
                        }
                    }
                    created = thread_created_rx.recv(), if listen_for_threads => {
                        match created {
                            Ok(thread_id) => {
                                let mut initialized_connection_ids = Vec::new();
                                for (connection_id, connection_state) in &connections {
                                    if connection_state.session.initialized() {
                                        initialized_connection_ids.push(*connection_id);
                                    }
                                }
                                processor
                                    .try_attach_thread_listener(
                                        thread_id,
                                        initialized_connection_ids,
                                    )
                                    .await;
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                                // TODO(jif) handle lag.
                                // Assumes thread creation volume is low enough that lag never happens.
                                // If it does, we log and continue without resyncing to avoid attaching
                                // listeners for threads that should remain unsubscribed.
                                warn!("thread_created receiver lagged; skipping resync");
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                                listen_for_threads = false;
                            }
                        }
                    }
                }
            }

            if !shutdown_state.forced() {
                processor.drain_background_tasks().await;
                processor.shutdown_threads().await;
            }
            info!("processor task exited (channel closed)");
        }
    });
    record_startup_trace_event("app_server.processor.ready");

    drop(transport_event_tx);

    let _ = processor_handle.await;
    let _ = outbound_handle.await;

    transport_shutdown_token.cancel();
    for handle in transport_accept_handles {
        let _ = handle.await;
    }

    if let Some(otel) = otel {
        otel.shutdown();
    }

    Ok(())
}

fn analytics_rpc_transport(transport: AppServerTransport) -> AppServerRpcTransport {
    match transport {
        AppServerTransport::Stdio => AppServerRpcTransport::Stdio,
        AppServerTransport::WebSocket { .. } | AppServerTransport::Off => {
            AppServerRpcTransport::Websocket
        }
    }
}

#[cfg(test)]
mod tests {
    use super::LogFormat;
    use super::disabled_project_config_warning_entries_with_home;
    use super::project_warning_display_path;
    use super::runtime_auth_manager;
    use super::should_arm_idle_shutdown;
    use super::start_runtime_remote_control;
    use crate::transport::CHANNEL_CAPACITY;
    use crate::transport::TransportEvent;
    use crate::transport::persist_remote_control_enrollment_for_tests;
    use codex_app_server_protocol::ConfigLayerSource;
    use codex_config::ConfigLayerEntry;
    use codex_config::ConfigLayerStack;
    use codex_config::LoaderOverrides;
    use codex_core::config::ConfigBuilder;
    use codex_core::config::ConfigOverrides;
    use codex_core::test_support::auth_manager_from_auth_with_home;
    use codex_login::CodexAuth;
    use codex_state::StateRuntime;
    use codex_utils_absolute_path::AbsolutePathBuf;
    use pretty_assertions::assert_eq;
    use std::collections::BTreeMap;
    use std::sync::Arc;
    use tempfile::tempdir;
    use tokio::net::TcpListener;
    use tokio::net::TcpStream;
    use tokio::sync::mpsc;
    use tokio::sync::oneshot;
    use tokio::time::Duration;
    use tokio::time::timeout;
    use tokio_tungstenite::WebSocketStream;
    use tokio_tungstenite::accept_hdr_async;
    use tokio_tungstenite::tungstenite;
    use tokio_util::sync::CancellationToken;
    use toml::Value as TomlValue;

    #[test]
    fn log_format_from_env_value_matches_json_values_case_insensitively() {
        assert_eq!(LogFormat::from_env_value(Some("json")), LogFormat::Json);
        assert_eq!(LogFormat::from_env_value(Some("JSON")), LogFormat::Json);
        assert_eq!(LogFormat::from_env_value(Some("  Json  ")), LogFormat::Json);
    }

    #[test]
    fn log_format_from_env_value_defaults_for_non_json_values() {
        assert_eq!(
            LogFormat::from_env_value(/*value*/ None),
            LogFormat::Default
        );
        assert_eq!(LogFormat::from_env_value(Some("")), LogFormat::Default);
        assert_eq!(LogFormat::from_env_value(Some("text")), LogFormat::Default);
        assert_eq!(LogFormat::from_env_value(Some("jsonl")), LogFormat::Default);
    }

    #[test]
    fn idle_shutdown_only_arms_after_first_connection_and_without_running_turns() {
        assert!(should_arm_idle_shutdown(
            /*has_seen_connection*/ true, /*connection_count*/ 0,
            /*running_turn_count*/ 0,
        ));
        assert!(!should_arm_idle_shutdown(
            /*has_seen_connection*/ true, /*connection_count*/ 1,
            /*running_turn_count*/ 0,
        ));
        assert!(!should_arm_idle_shutdown(
            /*has_seen_connection*/ true, /*connection_count*/ 0,
            /*running_turn_count*/ 1,
        ));
        assert!(!should_arm_idle_shutdown(
            /*has_seen_connection*/ false, /*connection_count*/ 0,
            /*running_turn_count*/ 0,
        ));
    }

    #[test]
    fn project_warning_display_path_collapses_any_path_inside_dot_codex() {
        let dot_codex_folder = tempdir().expect("temp dir");
        let config_path = dot_codex_folder.path().join("project/.codex/config.toml");
        std::fs::create_dir_all(
            config_path
                .parent()
                .expect("config path should have a parent"),
        )
        .expect("create project .codex");

        assert_eq!(
            project_warning_display_path(&config_path),
            dot_codex_folder
                .path()
                .join("project")
                .display()
                .to_string()
        );
    }

    #[tokio::test]
    async fn disabled_project_config_warning_entries_skip_legacy_codex_home() {
        let home = tempdir().expect("temp home");
        let codex_home = home.path().join(".openinterpreter");
        let legacy_codex_home = home.path().join(".codex");
        std::fs::create_dir_all(&codex_home).expect("create interpreter home");
        std::fs::create_dir_all(&legacy_codex_home).expect("create legacy codex home");
        std::fs::create_dir_all(home.path().join("project").join(".codex"))
            .expect("create project codex");

        let mut config = ConfigBuilder::default()
            .loader_overrides(LoaderOverrides::without_managed_config_for_tests())
            .codex_home(codex_home.clone())
            .harness_overrides(ConfigOverrides {
                cwd: Some(home.path().join("project")),
                ..Default::default()
            })
            .build()
            .await
            .expect("build config");

        let legacy_layer = ConfigLayerEntry::new_disabled(
            ConfigLayerSource::Project {
                dot_codex_folder: AbsolutePathBuf::try_from(legacy_codex_home.clone())
                    .expect("absolute legacy codex home"),
            },
            TomlValue::Table(Default::default()),
            "legacy disabled",
        );
        let project_layer = ConfigLayerEntry::new_disabled(
            ConfigLayerSource::Project {
                dot_codex_folder: AbsolutePathBuf::try_from(home.path().join("project/.codex"))
                    .expect("absolute project .codex"),
            },
            TomlValue::Table(Default::default()),
            "project disabled",
        );
        config.config_layer_stack = ConfigLayerStack::new(
            vec![legacy_layer, project_layer],
            Default::default(),
            Default::default(),
        )
        .expect("config layer stack");

        let entries = disabled_project_config_warning_entries_with_home(&config, Some(home.path()));

        assert_eq!(
            entries,
            vec![(
                home.path().join("project").display().to_string(),
                "project disabled".to_string(),
            )]
        );
    }

    #[tokio::test]
    async fn runtime_auth_manager_honors_enable_codex_api_key_env() {
        let codex_home = tempdir().expect("temp codex home");
        let config = ConfigBuilder::default()
            .loader_overrides(LoaderOverrides::without_managed_config_for_tests())
            .codex_home(codex_home.path().to_path_buf())
            .build()
            .await
            .expect("build config");

        assert!(
            runtime_auth_manager(&config, /*enable_codex_api_key_env*/ true)
                .codex_api_key_env_enabled()
        );
        assert!(
            !runtime_auth_manager(&config, /*enable_codex_api_key_env*/ false)
                .codex_api_key_env_enabled()
        );
    }

    #[tokio::test]
    async fn runtime_remote_control_waits_for_stdio_client_name() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let remote_control_url = runtime_remote_control_url_for_listener(&listener);
        let codex_home = tempdir().expect("temp codex home");
        let config = ConfigBuilder::default()
            .loader_overrides(LoaderOverrides::without_managed_config_for_tests())
            .codex_home(codex_home.path().to_path_buf())
            .cli_overrides(vec![
                (
                    "chatgpt_base_url".to_string(),
                    TomlValue::String(remote_control_url.clone()),
                ),
                (
                    "features.remote_control".to_string(),
                    TomlValue::Boolean(true),
                ),
            ])
            .build()
            .await
            .expect("build config");
        let state_db =
            StateRuntime::init(config.sqlite_home.clone(), config.model_provider_id.clone())
                .await
                .expect("state runtime should initialize");
        let app_server_client_name = "stdio-client";
        let expected_server_id = "srv_e_persisted".to_string();
        persist_remote_control_enrollment_for_tests(
            Some(state_db.as_ref()),
            &remote_control_url,
            "account_id",
            Some(app_server_client_name),
            expected_server_id.as_str(),
            "env_persisted",
            "persisted-server",
        )
        .await
        .expect("persisted enrollment should save");

        let auth_manager = auth_manager_from_auth_with_home(
            CodexAuth::create_dummy_chatgpt_auth_for_testing(),
            codex_home.path().to_path_buf(),
        );
        let (transport_event_tx, _transport_event_rx) =
            mpsc::channel::<TransportEvent>(CHANNEL_CAPACITY);
        let (app_server_client_name_tx, app_server_client_name_rx) = oneshot::channel::<String>();
        let shutdown_token = CancellationToken::new();
        let (remote_task, _remote_control_handle) = start_runtime_remote_control(
            &config,
            Some(state_db.clone()),
            auth_manager,
            transport_event_tx,
            shutdown_token.clone(),
            Some(app_server_client_name_rx),
        )
        .await
        .expect("remote control should start");

        timeout(Duration::from_millis(100), listener.accept())
            .await
            .expect_err("remote control should wait for the stdio client name");

        let _ = app_server_client_name_tx.send(app_server_client_name.to_string());
        let (handshake_request, backend_websocket) =
            accept_runtime_remote_control_backend_connection(&listener).await;
        assert_eq!(
            handshake_request.path,
            "/backend-api/wham/remote/control/server"
        );
        assert_eq!(
            handshake_request.headers.get("x-codex-server-id"),
            Some(&expected_server_id)
        );
        let _backend_websocket = backend_websocket;

        shutdown_token.cancel();
        let _ = remote_task.await;
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct CapturedWebSocketRequest {
        path: String,
        headers: BTreeMap<String, String>,
    }

    fn runtime_remote_control_url_for_listener(listener: &TcpListener) -> String {
        let addr = listener
            .local_addr()
            .expect("listener should have a local address");
        format!("http://{addr}/backend-api/")
    }

    async fn accept_runtime_remote_control_backend_connection(
        listener: &TcpListener,
    ) -> (CapturedWebSocketRequest, WebSocketStream<TcpStream>) {
        let (stream, _) = timeout(Duration::from_secs(5), listener.accept())
            .await
            .expect("websocket request should arrive in time")
            .expect("listener accept should succeed");
        let captured_request = Arc::new(std::sync::Mutex::new(None::<CapturedWebSocketRequest>));
        let captured_request_for_callback = captured_request.clone();
        let websocket = accept_hdr_async(
            stream,
            move |request: &tungstenite::handshake::server::Request,
                  response: tungstenite::handshake::server::Response| {
                let headers = request
                    .headers()
                    .iter()
                    .map(|(name, value)| {
                        (
                            name.as_str().to_ascii_lowercase(),
                            value
                                .to_str()
                                .expect("header should be valid utf-8")
                                .to_string(),
                        )
                    })
                    .collect::<BTreeMap<_, _>>();
                *captured_request_for_callback
                    .lock()
                    .expect("capture lock should acquire") = Some(CapturedWebSocketRequest {
                    path: request.uri().path().to_string(),
                    headers,
                });
                Ok(response)
            },
        )
        .await
        .expect("websocket handshake should succeed");
        let captured_request = captured_request
            .lock()
            .expect("capture lock should acquire")
            .clone()
            .expect("websocket request should be captured");
        (captured_request, websocket)
    }
}
