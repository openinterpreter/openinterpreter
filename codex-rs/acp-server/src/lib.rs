#![recursion_limit = "256"]

use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use acp::Agent;
use acp::ByteStreams;
use acp::Client;
use acp::ConnectTo;
use acp::ConnectionTo;
use acp::Error;
use acp::schema::AgentAuthCapabilities;
use acp::schema::AgentCapabilities;
use acp::schema::AuthEnvVar;
use acp::schema::AuthMethod;
use acp::schema::AuthMethodAgent;
use acp::schema::AuthMethodEnvVar;
use acp::schema::AuthMethodId;
use acp::schema::AuthenticateRequest;
use acp::schema::AuthenticateResponse;
use acp::schema::CancelNotification;
use acp::schema::CloseSessionRequest;
use acp::schema::CloseSessionResponse;
use acp::schema::ContentBlock;
use acp::schema::ContentChunk;
use acp::schema::EmbeddedResource;
use acp::schema::EmbeddedResourceResource;
use acp::schema::Implementation;
use acp::schema::InitializeRequest;
use acp::schema::InitializeResponse;
use acp::schema::ListSessionsRequest;
use acp::schema::ListSessionsResponse;
use acp::schema::LoadSessionRequest;
use acp::schema::LoadSessionResponse;
use acp::schema::LogoutCapabilities;
use acp::schema::McpCapabilities;
use acp::schema::NewSessionRequest;
use acp::schema::NewSessionResponse;
use acp::schema::PermissionOption;
use acp::schema::PermissionOptionKind;
use acp::schema::Plan;
use acp::schema::PlanEntry;
use acp::schema::PlanEntryPriority;
use acp::schema::PlanEntryStatus;
use acp::schema::PromptCapabilities;
use acp::schema::PromptRequest;
use acp::schema::PromptResponse;
use acp::schema::ProtocolVersion;
use acp::schema::RequestPermissionOutcome;
use acp::schema::RequestPermissionRequest;
use acp::schema::SelectedPermissionOutcome;
use acp::schema::SessionCapabilities;
use acp::schema::SessionCloseCapabilities;
use acp::schema::SessionConfigId;
use acp::schema::SessionConfigOption;
use acp::schema::SessionConfigOptionCategory;
use acp::schema::SessionConfigOptionValue;
use acp::schema::SessionConfigSelectOption;
use acp::schema::SessionId;
use acp::schema::SessionInfo;
use acp::schema::SessionListCapabilities;
use acp::schema::SessionMode;
use acp::schema::SessionModeId;
use acp::schema::SessionModeState;
use acp::schema::SessionModelState;
use acp::schema::SessionNotification;
use acp::schema::SessionUpdate;
use acp::schema::SetSessionConfigOptionRequest;
use acp::schema::SetSessionConfigOptionResponse;
use acp::schema::SetSessionModeRequest;
use acp::schema::SetSessionModeResponse;
use acp::schema::SetSessionModelRequest;
use acp::schema::SetSessionModelResponse;
use acp::schema::StopReason;
use acp::schema::TextResourceContents;
use acp::schema::ToolCall;
use acp::schema::ToolCallContent;
use acp::schema::ToolCallId;
use acp::schema::ToolCallStatus;
use acp::schema::ToolCallUpdate;
use acp::schema::ToolCallUpdateFields;
use acp::schema::ToolKind;
use agent_client_protocol as acp;
use codex_app_server_client::AppServerEvent;
use codex_app_server_client::DEFAULT_IN_PROCESS_CHANNEL_CAPACITY;
use codex_app_server_client::EnvironmentManager;
use codex_app_server_client::ExecServerRuntimePaths;
use codex_app_server_client::InProcessAppServerClient;
use codex_app_server_client::InProcessAppServerRequestHandle;
use codex_app_server_client::InProcessClientStartArgs;
use codex_app_server_protocol::AskForApproval;
use codex_app_server_protocol::ClientRequest;
use codex_app_server_protocol::CommandExecutionApprovalDecision;
use codex_app_server_protocol::CommandExecutionRequestApprovalResponse;
use codex_app_server_protocol::ConfigWarningNotification;
use codex_app_server_protocol::FileChangeApprovalDecision;
use codex_app_server_protocol::FileChangeRequestApprovalResponse;
use codex_app_server_protocol::GetAccountParams;
use codex_app_server_protocol::GetAccountResponse;
use codex_app_server_protocol::ImageReference;
use codex_app_server_protocol::InterpreterHarness;
use codex_app_server_protocol::InterpreterHarnessListParams;
use codex_app_server_protocol::InterpreterHarnessListResponse;
use codex_app_server_protocol::JSONRPCErrorError;
use codex_app_server_protocol::ModelListParams;
use codex_app_server_protocol::ModelListResponse;
use codex_app_server_protocol::RequestId;
use codex_app_server_protocol::SandboxPolicy;
use codex_app_server_protocol::ServerNotification;
use codex_app_server_protocol::ServerRequest;
use codex_app_server_protocol::ThreadForkParams;
use codex_app_server_protocol::ThreadForkResponse;
use codex_app_server_protocol::ThreadListParams;
use codex_app_server_protocol::ThreadListResponse;
use codex_app_server_protocol::ThreadResumeParams;
use codex_app_server_protocol::ThreadResumeResponse;
use codex_app_server_protocol::ThreadSortKey;
use codex_app_server_protocol::ThreadSourceKind;
use codex_app_server_protocol::ThreadStartParams;
use codex_app_server_protocol::ThreadStartResponse;
use codex_app_server_protocol::Turn;
use codex_app_server_protocol::TurnCompletedNotification;
use codex_app_server_protocol::TurnInterruptParams;
use codex_app_server_protocol::TurnInterruptResponse;
use codex_app_server_protocol::TurnPlanStepStatus;
use codex_app_server_protocol::TurnStartParams;
use codex_app_server_protocol::TurnStartResponse;
use codex_app_server_protocol::TurnStatus;
use codex_app_server_protocol::UserInput;
use codex_arg0::Arg0DispatchPaths;
use codex_config::CloudConfigBundleLoader;
use codex_config::LoaderOverrides;
use codex_core::config::Config;
use codex_feedback::CodexFeedback;
use codex_login::CODEX_API_KEY_ENV_VAR;
use codex_login::OPENAI_API_KEY_ENV_VAR;
use codex_protocol::openai_models::ReasoningEffort;
use codex_protocol::protocol::SessionSource;
use serde::Serialize;
use tokio::io;
use tokio::sync::Mutex;
use tokio_util::compat::TokioAsyncReadCompatExt as _;
use tokio_util::compat::TokioAsyncWriteCompatExt as _;

const CONFIG_ID_MODEL: &str = "model";
const CONFIG_ID_REASONING_EFFORT: &str = "reasoning_effort";
const CONFIG_ID_HARNESS: &str = "harness";

pub async fn run_main(arg0_paths: Arg0DispatchPaths) -> anyhow::Result<()> {
    init_tracing();
    let agent = Arc::new(AppServerAcpAgent::start(arg0_paths).await?);
    let outgoing = io::stdout().compat_write();
    let incoming = io::stdin().compat();
    agent.serve(ByteStreams::new(outgoing, incoming)).await?;
    Ok(())
}

fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .try_init();
}

#[derive(Clone)]
struct SessionState {
    thread_id: String,
    cwd: PathBuf,
    model: String,
    model_provider: String,
    reasoning_effort: Option<ReasoningEffort>,
    harness: Option<String>,
    mode: AcpSessionMode,
    active_turn_id: Option<String>,
}

#[derive(Clone, Copy)]
enum AcpSessionMode {
    ReadOnly,
    WorkspaceWrite,
    FullAccess,
}

impl AcpSessionMode {
    fn from_mode_id(mode_id: &SessionModeId) -> Result<Self, Error> {
        match mode_id.0.as_ref() {
            "read-only" => Ok(Self::ReadOnly),
            "workspace-write" => Ok(Self::WorkspaceWrite),
            "full-access" => Ok(Self::FullAccess),
            _ => Err(Error::invalid_params().data("unknown mode")),
        }
    }

    fn mode_id(self) -> SessionModeId {
        match self {
            Self::ReadOnly => SessionModeId::new("read-only"),
            Self::WorkspaceWrite => SessionModeId::new("workspace-write"),
            Self::FullAccess => SessionModeId::new("full-access"),
        }
    }

    fn approval_policy(self) -> AskForApproval {
        match self {
            Self::ReadOnly | Self::WorkspaceWrite => AskForApproval::UnlessTrusted,
            Self::FullAccess => AskForApproval::Never,
        }
    }

    fn sandbox_policy(self, cwd: &Path) -> Result<SandboxPolicy, Error> {
        match self {
            Self::ReadOnly => Ok(SandboxPolicy::ReadOnly {
                network_access: false,
            }),
            Self::WorkspaceWrite => {
                Ok(SandboxPolicy::WorkspaceWrite {
                    writable_roots: vec![cwd.to_path_buf().try_into().map_err(|_| {
                        Error::invalid_params().data("session cwd must be absolute")
                    })?],
                    network_access: false,
                    exclude_tmpdir_env_var: false,
                    exclude_slash_tmp: false,
                })
            }
            Self::FullAccess => Ok(SandboxPolicy::DangerFullAccess),
        }
    }
}

struct AppServerAcpAgent {
    client: InProcessAppServerRequestHandle,
    event_rx: async_channel::Receiver<AppServerEvent>,
    sessions: Mutex<HashMap<SessionId, SessionState>>,
    config: Arc<Config>,
}

impl AppServerAcpAgent {
    async fn start(arg0_paths: Arg0DispatchPaths) -> anyhow::Result<Self> {
        let config = Config::load_with_cli_overrides(Vec::new()).await?;
        let config_warnings = config
            .startup_warnings
            .iter()
            .map(|summary| ConfigWarningNotification {
                summary: summary.clone(),
                details: None,
                path: None,
                range: None,
            })
            .collect();
        let runtime_paths = ExecServerRuntimePaths::from_optional_paths(
            arg0_paths.codex_self_exe.clone(),
            arg0_paths.codex_linux_sandbox_exe.clone(),
        )?;
        let environment_manager = Arc::new(
            EnvironmentManager::from_codex_home(
                config.codex_home.clone(),
                Some(runtime_paths),
                config.http_client_factory(),
            )
            .await?,
        );
        let state_db = codex_core::init_state_db(&config).await;
        let mut client = InProcessAppServerClient::start(InProcessClientStartArgs {
            arg0_paths,
            config: Arc::new(config.clone()),
            cli_overrides: Vec::new(),
            loader_overrides: LoaderOverrides::default(),
            strict_config: false,
            cloud_config_bundle: CloudConfigBundleLoader::default(),
            feedback: CodexFeedback::new(),
            log_db: None,
            state_db,
            environment_manager,
            config_warnings,
            session_source: SessionSource::Unknown,
            enable_codex_api_key_env: true,
            client_name: "codex-acp".to_string(),
            client_version: codex_product_info::Product::current()
                .codex_compatibility_version()
                .to_string(),
            experimental_api: true,
            mcp_server_openai_form_elicitation: false,
            opt_out_notification_methods: Vec::new(),
            channel_capacity: DEFAULT_IN_PROCESS_CHANNEL_CAPACITY,
        })
        .await?;
        let request_handle = client.request_handle();
        let (event_tx, event_rx) = async_channel::bounded(DEFAULT_IN_PROCESS_CHANNEL_CAPACITY);
        tokio::spawn(async move {
            while let Some(event) = client.next_event().await {
                if event_tx.send(event.into()).await.is_err() {
                    break;
                }
            }
        });

        Ok(Self {
            client: request_handle,
            event_rx,
            sessions: Mutex::new(HashMap::new()),
            config: Arc::new(config),
        })
    }

    async fn serve(self: Arc<Self>, transport: impl ConnectTo<Agent> + 'static) -> acp::Result<()> {
        let agent = self;
        Agent
            .builder()
            .name("codex-acp")
            .on_receive_request(
                {
                    let agent = agent.clone();
                    async move |request: InitializeRequest, responder, _cx| {
                        responder.respond_with_result(agent.initialize(request).await)
                    }
                },
                acp::on_receive_request!(),
            )
            .on_receive_request(
                {
                    let agent = agent.clone();
                    async move |request: AuthenticateRequest,
                                responder,
                                cx: ConnectionTo<Client>| {
                        let agent = agent.clone();
                        cx.spawn(async move {
                            responder.respond_with_result(agent.authenticate(request).await)
                        })?;
                        Ok(())
                    }
                },
                acp::on_receive_request!(),
            )
            .on_receive_request(
                {
                    let agent = agent.clone();
                    async move |request: NewSessionRequest, responder, cx: ConnectionTo<Client>| {
                        let agent = agent.clone();
                        cx.spawn(async move {
                            responder.respond_with_result(agent.new_session(request).await)
                        })?;
                        Ok(())
                    }
                },
                acp::on_receive_request!(),
            )
            .on_receive_request(
                {
                    let agent = agent.clone();
                    async move |request: LoadSessionRequest, responder, cx: ConnectionTo<Client>| {
                        let agent = agent.clone();
                        let request_cx = cx.clone();
                        cx.spawn(async move {
                            responder
                                .respond_with_result(agent.load_session(request, request_cx).await)
                        })?;
                        Ok(())
                    }
                },
                acp::on_receive_request!(),
            )
            .on_receive_request(
                {
                    let agent = agent.clone();
                    async move |request: ListSessionsRequest,
                                responder,
                                cx: ConnectionTo<Client>| {
                        let agent = agent.clone();
                        cx.spawn(async move {
                            responder.respond_with_result(agent.list_sessions(request).await)
                        })?;
                        Ok(())
                    }
                },
                acp::on_receive_request!(),
            )
            .on_receive_request(
                {
                    let agent = agent.clone();
                    async move |request: CloseSessionRequest,
                                responder,
                                cx: ConnectionTo<Client>| {
                        let agent = agent.clone();
                        cx.spawn(async move {
                            responder.respond_with_result(agent.close_session(request).await)
                        })?;
                        Ok(())
                    }
                },
                acp::on_receive_request!(),
            )
            .on_receive_request(
                {
                    let agent = agent.clone();
                    async move |request: PromptRequest, responder, cx: ConnectionTo<Client>| {
                        let agent = agent.clone();
                        let request_cx = cx.clone();
                        cx.spawn(async move {
                            responder.respond_with_result(agent.prompt(request, request_cx).await)
                        })?;
                        Ok(())
                    }
                },
                acp::on_receive_request!(),
            )
            .on_receive_notification(
                {
                    let agent = agent.clone();
                    async move |notification: CancelNotification, cx: ConnectionTo<Client>| {
                        let agent = agent.clone();
                        cx.spawn(async move { agent.cancel(notification).await })?;
                        Ok(())
                    }
                },
                acp::on_receive_notification!(),
            )
            .on_receive_request(
                {
                    let agent = agent.clone();
                    async move |request: SetSessionModeRequest,
                                responder,
                                cx: ConnectionTo<Client>| {
                        let agent = agent.clone();
                        cx.spawn(async move {
                            responder.respond_with_result(agent.set_session_mode(request).await)
                        })?;
                        Ok(())
                    }
                },
                acp::on_receive_request!(),
            )
            .on_receive_request(
                {
                    let agent = agent.clone();
                    async move |request: SetSessionModelRequest,
                                responder,
                                cx: ConnectionTo<Client>| {
                        let agent = agent.clone();
                        cx.spawn(async move {
                            responder.respond_with_result(agent.set_session_model(request).await)
                        })?;
                        Ok(())
                    }
                },
                acp::on_receive_request!(),
            )
            .on_receive_request(
                {
                    let agent = agent.clone();
                    async move |request: SetSessionConfigOptionRequest,
                                responder,
                                cx: ConnectionTo<Client>| {
                        let agent = agent.clone();
                        cx.spawn(async move {
                            responder
                                .respond_with_result(agent.set_session_config_option(request).await)
                        })?;
                        Ok(())
                    }
                },
                acp::on_receive_request!(),
            )
            .connect_to(transport)
            .await
    }

    async fn initialize(&self, _request: InitializeRequest) -> Result<InitializeResponse, Error> {
        let mut agent_capabilities = AgentCapabilities::new()
            .prompt_capabilities(PromptCapabilities::new().embedded_context(true).image(true))
            .mcp_capabilities(McpCapabilities::new().http(false).sse(false))
            .load_session(true)
            .auth(AgentAuthCapabilities::new().logout(LogoutCapabilities::new()));
        agent_capabilities.session_capabilities = SessionCapabilities::new()
            .close(SessionCloseCapabilities::new())
            .list(SessionListCapabilities::new());

        Ok(InitializeResponse::new(ProtocolVersion::V1)
            .agent_info(
                Implementation::new("codex-acp", env!("CARGO_PKG_VERSION"))
                    .title(codex_product_info::Product::current().display_name()),
            )
            .agent_capabilities(agent_capabilities)
            .auth_methods(vec![
                AuthMethod::Agent(
                    AuthMethodAgent::new(AuthMethodId::new("chatgpt"), "Login with ChatGPT")
                        .description(format!(
                            "Use your ChatGPT login with {}",
                            codex_product_info::Product::current().display_name()
                        )),
                ),
                AuthMethod::EnvVar(AuthMethodEnvVar::new(
                    AuthMethodId::new("codex-api-key"),
                    format!("Use {CODEX_API_KEY_ENV_VAR}"),
                    vec![AuthEnvVar::new(CODEX_API_KEY_ENV_VAR)],
                )),
                AuthMethod::EnvVar(AuthMethodEnvVar::new(
                    AuthMethodId::new("openai-api-key"),
                    format!("Use {OPENAI_API_KEY_ENV_VAR}"),
                    vec![AuthEnvVar::new(OPENAI_API_KEY_ENV_VAR)],
                )),
            ]))
    }

    async fn authenticate(
        &self,
        _request: AuthenticateRequest,
    ) -> Result<AuthenticateResponse, Error> {
        let response: GetAccountResponse = self
            .client
            .request_typed(ClientRequest::GetAccount {
                request_id: next_request_id(),
                params: GetAccountParams {
                    refresh_token: false,
                },
            })
            .await
            .map_err(app_server_error)?;
        if response.account.is_some() || !response.requires_openai_auth {
            Ok(AuthenticateResponse::new())
        } else {
            Err(Error::auth_required().data(format!(
                "{} is not authenticated. Run `{} login` or configure an API key.",
                codex_product_info::Product::current().short_display_name(),
                codex_product_info::Product::current().command_name(),
            )))
        }
    }

    async fn new_session(&self, request: NewSessionRequest) -> Result<NewSessionResponse, Error> {
        let response: ThreadStartResponse = {
            self.client
                .request_typed(ClientRequest::ThreadStart {
                    request_id: next_request_id(),
                    params: ThreadStartParams {
                        cwd: Some(request.cwd.to_string_lossy().to_string()),
                        service_name: Some("ACP".to_string()),
                        ..ThreadStartParams::default()
                    },
                })
                .await
                .map_err(app_server_error)?
        };

        let session_id = SessionId::new(response.thread.id.clone());
        let session = SessionState {
            thread_id: response.thread.id,
            cwd: response.cwd.into_path_buf(),
            model: response.model,
            model_provider: response.model_provider,
            reasoning_effort: response.reasoning_effort,
            harness: self.config.harness.clone(),
            mode: AcpSessionMode::WorkspaceWrite,
            active_turn_id: None,
        };
        self.sessions
            .lock()
            .await
            .insert(session_id.clone(), session.clone());

        Ok(NewSessionResponse::new(session_id)
            .modes(session_modes())
            .models(self.session_models().await.ok())
            .config_options(Some(self.config_options(&session).await?)))
    }

    async fn load_session(
        &self,
        request: LoadSessionRequest,
        cx: ConnectionTo<Client>,
    ) -> Result<LoadSessionResponse, Error> {
        let response: ThreadResumeResponse = {
            self.client
                .request_typed(ClientRequest::ThreadResume {
                    request_id: next_request_id(),
                    params: ThreadResumeParams {
                        thread_id: request.session_id.0.to_string(),
                        cwd: Some(request.cwd.to_string_lossy().to_string()),
                        ..ThreadResumeParams::default()
                    },
                })
                .await
                .map_err(app_server_error)?
        };

        let session_id = SessionId::new(response.thread.id.clone());
        let session = SessionState {
            thread_id: response.thread.id.clone(),
            cwd: response.cwd.into_path_buf(),
            model: response.model,
            model_provider: response.model_provider,
            reasoning_effort: response.reasoning_effort,
            harness: self.config.harness.clone(),
            mode: AcpSessionMode::WorkspaceWrite,
            active_turn_id: None,
        };
        self.sessions
            .lock()
            .await
            .insert(session_id.clone(), session.clone());
        replay_thread_history(session_id, response.thread.turns, &cx)?;

        Ok(LoadSessionResponse::new()
            .modes(session_modes())
            .models(self.session_models().await.ok())
            .config_options(Some(self.config_options(&session).await?)))
    }

    async fn list_sessions(
        &self,
        request: ListSessionsRequest,
    ) -> Result<ListSessionsResponse, Error> {
        let response: ThreadListResponse = {
            self.client
                .request_typed(ClientRequest::ThreadList {
                    request_id: next_request_id(),
                    params: ThreadListParams {
                        cursor: request.cursor.clone(),
                        limit: Some(25),
                        sort_key: Some(ThreadSortKey::UpdatedAt),
                        sort_direction: None,
                        model_providers: None,
                        source_kinds: Some(vec![
                            ThreadSourceKind::Cli,
                            ThreadSourceKind::VsCode,
                            ThreadSourceKind::Unknown,
                        ]),
                        originators: None,
                        archived: Some(false),
                        section_id: None,
                        project_id: None,
                        cwd: None,
                        use_state_db_only: false,
                        search_term: None,
                        parent_thread_id: None,
                        ancestor_thread_id: None,
                    },
                })
                .await
                .map_err(app_server_error)?
        };

        let sessions = response
            .data
            .into_iter()
            .map(|thread| {
                SessionInfo::new(SessionId::new(thread.id), thread.cwd.into_path_buf()).title(
                    thread.name.unwrap_or_else(|| {
                        format!(
                            "{} session",
                            codex_product_info::Product::current().short_display_name()
                        )
                    }),
                )
            })
            .collect();
        Ok(ListSessionsResponse::new(sessions).next_cursor(response.next_cursor))
    }

    async fn close_session(
        &self,
        request: CloseSessionRequest,
    ) -> Result<CloseSessionResponse, Error> {
        self.sessions.lock().await.remove(&request.session_id);
        Ok(CloseSessionResponse::new())
    }

    async fn prompt(
        &self,
        request: PromptRequest,
        cx: ConnectionTo<Client>,
    ) -> Result<PromptResponse, Error> {
        let session_id = request.session_id.clone();
        let (thread_id, cwd, model, effort, mode) = self.session_snapshot(&session_id).await?;
        let response: TurnStartResponse = {
            self.client
                .request_typed(ClientRequest::TurnStart {
                    request_id: next_request_id(),
                    params: TurnStartParams {
                        thread_id: thread_id.clone(),
                        input: build_prompt_items(request.prompt),
                        cwd: Some(cwd.clone()),
                        approval_policy: Some(mode.approval_policy()),
                        sandbox_policy: Some(mode.sandbox_policy(&cwd)?),
                        model: Some(model),
                        effort,
                        ..TurnStartParams::default()
                    },
                })
                .await
                .map_err(app_server_error)?
        };
        let turn_id = response.turn.id.clone();
        self.set_active_turn(&session_id, Some(turn_id.clone()))
            .await;
        let stop_reason = self
            .drain_turn_events(session_id.clone(), thread_id, turn_id, cx)
            .await?;
        self.set_active_turn(&session_id, /*turn_id*/ None).await;
        Ok(PromptResponse::new(stop_reason))
    }

    async fn cancel(&self, notification: CancelNotification) -> Result<(), Error> {
        let session = self
            .sessions
            .lock()
            .await
            .get(&notification.session_id)
            .cloned()
            .ok_or_else(|| Error::invalid_params().data("unknown session"))?;
        if let Some(turn_id) = session.active_turn_id {
            let _: TurnInterruptResponse = self
                .client
                .request_typed(ClientRequest::TurnInterrupt {
                    request_id: next_request_id(),
                    params: TurnInterruptParams {
                        thread_id: session.thread_id,
                        turn_id,
                    },
                })
                .await
                .map_err(app_server_error)?;
        }
        Ok(())
    }

    async fn set_session_mode(
        &self,
        request: SetSessionModeRequest,
    ) -> Result<SetSessionModeResponse, Error> {
        let mode = AcpSessionMode::from_mode_id(&request.mode_id)?;
        let mut sessions = self.sessions.lock().await;
        let session = sessions
            .get_mut(&request.session_id)
            .ok_or_else(|| Error::invalid_params().data("unknown session"))?;
        session.mode = mode;
        Ok(SetSessionModeResponse::default())
    }

    async fn set_session_model(
        &self,
        request: SetSessionModelRequest,
    ) -> Result<SetSessionModelResponse, Error> {
        let mut sessions = self.sessions.lock().await;
        let session = sessions
            .get_mut(&request.session_id)
            .ok_or_else(|| Error::invalid_params().data("unknown session"))?;
        session.model = request.model_id.0.to_string();
        Ok(SetSessionModelResponse::default())
    }

    async fn set_session_config_option(
        &self,
        request: SetSessionConfigOptionRequest,
    ) -> Result<SetSessionConfigOptionResponse, Error> {
        let session = self.session_state(&request.session_id).await?;
        match request.config_id.0.as_ref() {
            CONFIG_ID_MODEL => {
                let model = config_value_id(&request.value)
                    .ok_or_else(|| Error::invalid_params().data("missing model value"))?
                    .to_string();
                let mut sessions = self.sessions.lock().await;
                sessions
                    .get_mut(&request.session_id)
                    .ok_or_else(|| Error::invalid_params().data("unknown session"))?
                    .model = model;
            }
            CONFIG_ID_REASONING_EFFORT => {
                let reasoning_effort = parse_reasoning_effort(&request.value)?;
                let mut sessions = self.sessions.lock().await;
                sessions
                    .get_mut(&request.session_id)
                    .ok_or_else(|| Error::invalid_params().data("unknown session"))?
                    .reasoning_effort = reasoning_effort;
            }
            CONFIG_ID_HARNESS => {
                let requested_harness = config_value_id(&request.value)
                    .ok_or_else(|| Error::invalid_params().data("missing harness value"))?;
                let selected_harness = self
                    .interpreter_harnesses(&session)
                    .await?
                    .into_iter()
                    .find(|harness| harness.id.as_deref().unwrap_or_default() == requested_harness)
                    .map(|harness| harness.id);
                if selected_harness.is_none() && !requested_harness.is_empty() {
                    return Err(Error::invalid_params().data("unknown harness"));
                }

                let response: ThreadForkResponse = self
                    .client
                    .request_typed(ClientRequest::ThreadFork {
                        request_id: next_request_id(),
                        params: ThreadForkParams {
                            thread_id: session.thread_id.clone(),
                            config: Some(HashMap::from([(
                                "harness".to_string(),
                                serde_json::json!(selected_harness.clone()),
                            )])),
                            ..ThreadForkParams::default()
                        },
                    })
                    .await
                    .map_err(app_server_error)?;

                let mut sessions = self.sessions.lock().await;
                let session = sessions
                    .get_mut(&request.session_id)
                    .ok_or_else(|| Error::invalid_params().data("unknown session"))?;
                session.thread_id = response.thread.id;
                session.harness = selected_harness.flatten();
            }
            _ => return Err(Error::invalid_params().data("unknown config option")),
        }
        let session = self.session_state(&request.session_id).await?;
        Ok(SetSessionConfigOptionResponse::new(
            self.config_options(&session).await?,
        ))
    }

    async fn session_state(&self, session_id: &SessionId) -> Result<SessionState, Error> {
        self.sessions
            .lock()
            .await
            .get(session_id)
            .cloned()
            .ok_or_else(|| Error::invalid_params().data("unknown session"))
    }

    async fn session_snapshot(
        &self,
        session_id: &SessionId,
    ) -> Result<
        (
            String,
            PathBuf,
            String,
            Option<ReasoningEffort>,
            AcpSessionMode,
        ),
        Error,
    > {
        let sessions = self.sessions.lock().await;
        let session = sessions
            .get(session_id)
            .ok_or_else(|| Error::invalid_params().data("unknown session"))?;
        Ok((
            session.thread_id.clone(),
            session.cwd.clone(),
            session.model.clone(),
            session.reasoning_effort.clone(),
            session.mode,
        ))
    }

    async fn set_active_turn(&self, session_id: &SessionId, turn_id: Option<String>) {
        if let Some(session) = self.sessions.lock().await.get_mut(session_id) {
            session.active_turn_id = turn_id;
        }
    }

    async fn drain_turn_events(
        &self,
        session_id: SessionId,
        thread_id: String,
        turn_id: String,
        cx: ConnectionTo<Client>,
    ) -> Result<StopReason, Error> {
        let mut completed_item_ids = HashSet::new();
        loop {
            let event = self.event_rx.recv().await.ok();
            match event {
                Some(AppServerEvent::ServerNotification(notification)) => {
                    if let Some(stop_reason) = handle_notification(
                        &session_id,
                        &thread_id,
                        &turn_id,
                        *notification,
                        &cx,
                        &mut completed_item_ids,
                    )? {
                        return Ok(stop_reason);
                    }
                }
                Some(AppServerEvent::ServerRequest(request)) => {
                    self.handle_server_request(&session_id, *request, &cx)
                        .await?;
                }
                Some(AppServerEvent::Lagged { .. }) => {}
                Some(AppServerEvent::Disconnected { message }) => {
                    return Err(Error::internal_error().data(message));
                }
                None => return Err(Error::internal_error().data("app-server event stream closed")),
            }
        }
    }

    async fn handle_server_request(
        &self,
        session_id: &SessionId,
        request: ServerRequest,
        cx: &ConnectionTo<Client>,
    ) -> Result<(), Error> {
        match request {
            ServerRequest::CommandExecutionRequestApproval { request_id, params } => {
                let reason = params.reason.unwrap_or_else(|| {
                    format!(
                        "{} wants to run a command",
                        codex_product_info::Product::current().short_display_name()
                    )
                });
                let outcome = request_permission(
                    session_id,
                    cx,
                    "Run command",
                    &reason,
                    vec![
                        PermissionOption::new("accept", "Allow", PermissionOptionKind::AllowOnce),
                        PermissionOption::new(
                            "acceptForSession",
                            "Allow for session",
                            PermissionOptionKind::AllowAlways,
                        ),
                        PermissionOption::new("decline", "Deny", PermissionOptionKind::RejectOnce),
                    ],
                )
                .await?;
                let decision = match selected_outcome_id(outcome).as_deref() {
                    Some("accept") => CommandExecutionApprovalDecision::Accept,
                    Some("acceptForSession") => CommandExecutionApprovalDecision::AcceptForSession,
                    Some("decline") => CommandExecutionApprovalDecision::Decline,
                    _ => CommandExecutionApprovalDecision::Cancel,
                };
                self.resolve_server_request(
                    request_id,
                    CommandExecutionRequestApprovalResponse { decision },
                )
                .await?;
            }
            ServerRequest::FileChangeRequestApproval { request_id, params } => {
                let reason = params.reason.unwrap_or_else(|| {
                    format!(
                        "{} wants to edit files",
                        codex_product_info::Product::current().short_display_name()
                    )
                });
                let outcome = request_permission(
                    session_id,
                    cx,
                    "Apply file changes",
                    &reason,
                    vec![
                        PermissionOption::new("accept", "Allow", PermissionOptionKind::AllowOnce),
                        PermissionOption::new(
                            "acceptForSession",
                            "Allow for session",
                            PermissionOptionKind::AllowAlways,
                        ),
                        PermissionOption::new("decline", "Deny", PermissionOptionKind::RejectOnce),
                    ],
                )
                .await?;
                let decision = match selected_outcome_id(outcome).as_deref() {
                    Some("accept") => FileChangeApprovalDecision::Accept,
                    Some("acceptForSession") => FileChangeApprovalDecision::AcceptForSession,
                    Some("decline") => FileChangeApprovalDecision::Decline,
                    _ => FileChangeApprovalDecision::Cancel,
                };
                self.resolve_server_request(
                    request_id,
                    FileChangeRequestApprovalResponse { decision },
                )
                .await?;
            }
            other => {
                let request_id = other.id().clone();
                self.client
                    .reject_server_request(
                        request_id,
                        JSONRPCErrorError {
                            code: -32601,
                            message: "ACP server cannot satisfy this app-server request"
                                .to_string(),
                            data: None,
                        },
                    )
                    .await
                    .map_err(Error::into_internal_error)?;
            }
        }
        Ok(())
    }

    async fn resolve_server_request<T: Serialize>(
        &self,
        request_id: RequestId,
        response: T,
    ) -> Result<(), Error> {
        let result = serde_json::to_value(response).map_err(Error::into_internal_error)?;
        self.client
            .resolve_server_request(request_id, result)
            .await
            .map_err(Error::into_internal_error)
    }

    async fn session_models(&self) -> Result<SessionModelState, Error> {
        let response: ModelListResponse = {
            self.client
                .request_typed(ClientRequest::ModelList {
                    request_id: next_request_id(),
                    params: ModelListParams {
                        cursor: None,
                        limit: None,
                        include_hidden: Some(false),
                        model_provider: None,
                    },
                })
                .await
                .map_err(app_server_error)?
        };
        let current = response
            .data
            .iter()
            .find(|model| model.is_default)
            .or_else(|| response.data.first())
            .map(|model| model.model.clone())
            .unwrap_or_else(|| default_model(&self.config));
        let available = response
            .data
            .into_iter()
            .map(|model| {
                acp::schema::ModelInfo::new(model.model, model.display_name)
                    .description(model.description)
            })
            .collect();
        Ok(SessionModelState::new(
            acp::schema::ModelId::new(current),
            available,
        ))
    }

    async fn interpreter_harnesses(
        &self,
        session: &SessionState,
    ) -> Result<Vec<InterpreterHarness>, Error> {
        let response: InterpreterHarnessListResponse = self
            .client
            .request_typed(ClientRequest::InterpreterHarnessList {
                request_id: next_request_id(),
                params: InterpreterHarnessListParams {
                    provider_id: session.model_provider.clone(),
                    model: Some(session.model.clone()),
                },
            })
            .await
            .map_err(app_server_error)?;
        Ok(response.data)
    }

    async fn config_options(
        &self,
        session: &SessionState,
    ) -> Result<Vec<SessionConfigOption>, Error> {
        let harness_options: Vec<_> = self
            .interpreter_harnesses(session)
            .await?
            .into_iter()
            .map(|harness| {
                let harness_id = harness.id.unwrap_or_default();
                SessionConfigSelectOption::new(harness_id, harness.label)
                    .description(harness.description)
            })
            .collect();
        Ok(vec![
            SessionConfigOption::select(
                SessionConfigId::new(CONFIG_ID_MODEL),
                "Model",
                session.model.clone(),
                vec![SessionConfigSelectOption::new(
                    session.model.clone(),
                    session.model.clone(),
                )],
            )
            .category(SessionConfigOptionCategory::Model),
            SessionConfigOption::select(
                SessionConfigId::new(CONFIG_ID_REASONING_EFFORT),
                "Reasoning",
                session
                    .reasoning_effort
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_else(|| "default".to_string()),
                reasoning_options(),
            )
            .category(SessionConfigOptionCategory::ThoughtLevel),
            SessionConfigOption::select(
                SessionConfigId::new(CONFIG_ID_HARNESS),
                "Harness",
                session.harness.clone().unwrap_or_default(),
                harness_options,
            )
            .category(SessionConfigOptionCategory::Other("_harness".to_string())),
        ])
    }
}

async fn request_permission(
    session_id: &SessionId,
    cx: &ConnectionTo<Client>,
    title: &str,
    message: &str,
    options: Vec<PermissionOption>,
) -> Result<RequestPermissionOutcome, Error> {
    let tool_call = ToolCallUpdate::new(
        ToolCallId::new(format!(
            "permission:{}",
            title.to_ascii_lowercase().replace(' ', "-")
        )),
        ToolCallUpdateFields::new()
            .title(title.to_string())
            .status(ToolCallStatus::Pending)
            .content(vec![ToolCallContent::from(message.to_string())]),
    );
    let response = cx
        .send_request(RequestPermissionRequest::new(
            session_id.clone(),
            tool_call,
            options,
        ))
        .block_task()
        .await?;
    Ok(response.outcome)
}

fn selected_outcome_id(outcome: RequestPermissionOutcome) -> Option<String> {
    match outcome {
        RequestPermissionOutcome::Selected(SelectedPermissionOutcome { option_id, .. }) => {
            Some(option_id.0.to_string())
        }
        RequestPermissionOutcome::Cancelled => None,
        _ => None,
    }
}

fn handle_notification(
    session_id: &SessionId,
    thread_id: &str,
    turn_id: &str,
    notification: ServerNotification,
    cx: &ConnectionTo<Client>,
    completed_item_ids: &mut HashSet<String>,
) -> Result<Option<StopReason>, Error> {
    match notification {
        ServerNotification::AgentMessageDelta(payload)
            if payload.thread_id == thread_id && payload.turn_id == turn_id =>
        {
            send_update(
                session_id,
                cx,
                SessionUpdate::AgentMessageChunk(ContentChunk::new(payload.delta.into())),
            )?;
        }
        ServerNotification::ReasoningSummaryTextDelta(payload)
            if payload.thread_id == thread_id && payload.turn_id == turn_id =>
        {
            send_update(
                session_id,
                cx,
                SessionUpdate::AgentThoughtChunk(ContentChunk::new(payload.delta.into())),
            )?;
        }
        ServerNotification::ReasoningTextDelta(payload)
            if payload.thread_id == thread_id && payload.turn_id == turn_id =>
        {
            send_update(
                session_id,
                cx,
                SessionUpdate::AgentThoughtChunk(ContentChunk::new(payload.delta.into())),
            )?;
        }
        ServerNotification::TurnPlanUpdated(payload)
            if payload.thread_id == thread_id && payload.turn_id == turn_id =>
        {
            let entries = payload
                .plan
                .into_iter()
                .map(|step| {
                    PlanEntry::new(
                        step.step,
                        PlanEntryPriority::Medium,
                        match step.status {
                            TurnPlanStepStatus::Pending => PlanEntryStatus::Pending,
                            TurnPlanStepStatus::InProgress => PlanEntryStatus::InProgress,
                            TurnPlanStepStatus::Completed => PlanEntryStatus::Completed,
                        },
                    )
                })
                .collect();
            send_update(session_id, cx, SessionUpdate::Plan(Plan::new(entries)))?;
        }
        ServerNotification::ItemStarted(payload)
            if payload.thread_id == thread_id && payload.turn_id == turn_id =>
        {
            if let Some((tool_call_id, title, kind)) = tool_call_from_item(&payload.item) {
                send_update(
                    session_id,
                    cx,
                    SessionUpdate::ToolCall(
                        ToolCall::new(tool_call_id, title)
                            .kind(kind)
                            .status(ToolCallStatus::InProgress),
                    ),
                )?;
            }
        }
        ServerNotification::ItemCompleted(payload)
            if payload.thread_id == thread_id && payload.turn_id == turn_id =>
        {
            send_completed_item_update_if_new(session_id, cx, &payload.item, completed_item_ids)?;
        }
        ServerNotification::TurnCompleted(TurnCompletedNotification {
            thread_id: completed_thread_id,
            turn,
        }) if completed_thread_id == thread_id && turn.id == turn_id => {
            for item in &turn.items {
                send_completed_item_update_if_new(session_id, cx, item, completed_item_ids)?;
            }
            return Ok(Some(stop_reason_from_turn(turn)));
        }
        _ => {}
    }
    Ok(None)
}

fn send_update(
    session_id: &SessionId,
    cx: &ConnectionTo<Client>,
    update: SessionUpdate,
) -> Result<(), Error> {
    cx.send_notification(SessionNotification::new(session_id.clone(), update))
}

fn send_completed_item_update(
    session_id: &SessionId,
    cx: &ConnectionTo<Client>,
    item: &codex_app_server_protocol::ThreadItem,
) -> Result<(), Error> {
    match item {
        codex_app_server_protocol::ThreadItem::AgentMessage { text, .. } => {
            send_update(
                session_id,
                cx,
                SessionUpdate::AgentMessageChunk(ContentChunk::new(text.clone().into())),
            )?;
        }
        codex_app_server_protocol::ThreadItem::Reasoning {
            summary, content, ..
        } => {
            let text = summary
                .iter()
                .chain(content.iter())
                .cloned()
                .collect::<Vec<_>>()
                .join("\n");
            if !text.is_empty() {
                send_update(
                    session_id,
                    cx,
                    SessionUpdate::AgentThoughtChunk(ContentChunk::new(text.into())),
                )?;
            }
        }
        item => {
            if let Some((tool_call_id, title, kind)) = tool_call_from_item(item) {
                send_update(
                    session_id,
                    cx,
                    SessionUpdate::ToolCall(
                        ToolCall::new(tool_call_id, title)
                            .kind(kind)
                            .status(ToolCallStatus::Completed),
                    ),
                )?;
            }
        }
    }
    Ok(())
}

fn send_completed_item_update_if_new(
    session_id: &SessionId,
    cx: &ConnectionTo<Client>,
    item: &codex_app_server_protocol::ThreadItem,
    completed_item_ids: &mut HashSet<String>,
) -> Result<(), Error> {
    if completed_item_ids.insert(item.id().to_string()) {
        send_completed_item_update(session_id, cx, item)?;
    }
    Ok(())
}

fn stop_reason_from_turn(turn: Turn) -> StopReason {
    match turn.status {
        TurnStatus::Interrupted => StopReason::Cancelled,
        TurnStatus::Completed | TurnStatus::Failed | TurnStatus::InProgress => StopReason::EndTurn,
    }
}

fn tool_call_from_item(
    item: &codex_app_server_protocol::ThreadItem,
) -> Option<(ToolCallId, String, ToolKind)> {
    match item {
        codex_app_server_protocol::ThreadItem::CommandExecution { id, command, .. } => Some((
            ToolCallId::new(id.clone()),
            command.clone(),
            ToolKind::Execute,
        )),
        codex_app_server_protocol::ThreadItem::FileChange { id, .. } => Some((
            ToolCallId::new(id.clone()),
            "File change".to_string(),
            ToolKind::Edit,
        )),
        codex_app_server_protocol::ThreadItem::McpToolCall {
            id, server, tool, ..
        } => Some((
            ToolCallId::new(id.clone()),
            format!("{server}/{tool}"),
            ToolKind::Other,
        )),
        codex_app_server_protocol::ThreadItem::WebSearch(item) => Some((
            ToolCallId::new(item.id.clone()),
            "Web search".to_string(),
            ToolKind::Fetch,
        )),
        _ => None,
    }
}

fn replay_thread_history(
    session_id: SessionId,
    turns: Vec<Turn>,
    cx: &ConnectionTo<Client>,
) -> Result<(), Error> {
    for turn in turns {
        for item in turn.items {
            match item {
                codex_app_server_protocol::ThreadItem::UserMessage { content, .. } => {
                    for input in content {
                        if let UserInput::Text { text, .. } = input {
                            send_update(
                                &session_id,
                                cx,
                                SessionUpdate::UserMessageChunk(ContentChunk::new(text.into())),
                            )?;
                        }
                    }
                }
                codex_app_server_protocol::ThreadItem::AgentMessage { text, .. } => {
                    send_update(
                        &session_id,
                        cx,
                        SessionUpdate::AgentMessageChunk(ContentChunk::new(text.into())),
                    )?;
                }
                codex_app_server_protocol::ThreadItem::Reasoning {
                    summary, content, ..
                } => {
                    let text = summary
                        .into_iter()
                        .chain(content)
                        .collect::<Vec<_>>()
                        .join("\n");
                    if !text.is_empty() {
                        send_update(
                            &session_id,
                            cx,
                            SessionUpdate::AgentThoughtChunk(ContentChunk::new(text.into())),
                        )?;
                    }
                }
                item => {
                    if let Some((tool_call_id, title, kind)) = tool_call_from_item(&item) {
                        send_update(
                            &session_id,
                            cx,
                            SessionUpdate::ToolCall(
                                ToolCall::new(tool_call_id, title)
                                    .kind(kind)
                                    .status(ToolCallStatus::Completed),
                            ),
                        )?;
                    }
                }
            }
        }
    }
    Ok(())
}

fn session_modes() -> Option<SessionModeState> {
    Some(SessionModeState::new(
        AcpSessionMode::WorkspaceWrite.mode_id(),
        vec![
            SessionMode::new(SessionModeId::new("read-only"), "Read Only"),
            SessionMode::new(SessionModeId::new("workspace-write"), "Workspace Write"),
            SessionMode::new(SessionModeId::new("full-access"), "Full Access"),
        ],
    ))
}

fn build_prompt_items(prompt: Vec<ContentBlock>) -> Vec<UserInput> {
    prompt
        .into_iter()
        .filter_map(|block| match block {
            ContentBlock::Text(text_block) => Some(UserInput::Text {
                text: text_block.text,
                text_elements: Vec::new(),
            }),
            ContentBlock::Image(image_block) => Some(UserInput::Image {
                image: ImageReference::Inline {
                    url: format!("data:{};base64,{}", image_block.mime_type, image_block.data),
                },
                detail: None,
            }),
            ContentBlock::ResourceLink(link) => Some(UserInput::Text {
                text: format!("[{}]({})", link.name, link.uri),
                text_elements: Vec::new(),
            }),
            ContentBlock::Resource(EmbeddedResource {
                resource:
                    EmbeddedResourceResource::TextResourceContents(TextResourceContents {
                        text,
                        uri,
                        ..
                    }),
                ..
            }) => Some(UserInput::Text {
                text: format!("<context ref=\"{uri}\">\n{text}\n</context>"),
                text_elements: Vec::new(),
            }),
            ContentBlock::Audio(_) | ContentBlock::Resource(_) => None,
            _ => None,
        })
        .collect()
}

fn parse_reasoning_effort(
    value: &SessionConfigOptionValue,
) -> Result<Option<ReasoningEffort>, Error> {
    let Some(value) = config_value_id(value) else {
        return Ok(None);
    };
    match value {
        "minimal" => Ok(Some(ReasoningEffort::Minimal)),
        "low" => Ok(Some(ReasoningEffort::Low)),
        "medium" => Ok(Some(ReasoningEffort::Medium)),
        "high" => Ok(Some(ReasoningEffort::High)),
        "default" | "" => Ok(None),
        _ => Err(Error::invalid_params().data("unknown reasoning effort")),
    }
}

fn config_value_id(value: &SessionConfigOptionValue) -> Option<&str> {
    value.as_value_id().map(|value_id| value_id.0.as_ref())
}

fn default_model(config: &Config) -> String {
    config
        .model
        .clone()
        .unwrap_or_else(|| "gpt-5.1-codex".to_string())
}

fn reasoning_options() -> Vec<SessionConfigSelectOption> {
    [
        ("default", "Default"),
        ("minimal", "Minimal"),
        ("low", "Low"),
        ("medium", "Medium"),
        ("high", "High"),
    ]
    .into_iter()
    .map(|(value, label)| SessionConfigSelectOption::new(value, label))
    .collect()
}

fn app_server_error(err: codex_app_server_client::TypedRequestError) -> Error {
    Error::internal_error().data(err.to_string())
}

fn next_request_id() -> RequestId {
    use std::sync::atomic::AtomicI64;
    use std::sync::atomic::Ordering;

    static NEXT_REQUEST_ID: AtomicI64 = AtomicI64::new(1);
    RequestId::Integer(NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed))
}
