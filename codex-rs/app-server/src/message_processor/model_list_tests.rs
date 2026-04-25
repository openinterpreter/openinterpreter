use super::ConnectionSessionState;
use super::MessageProcessor;
use super::MessageProcessorArgs;
use crate::outgoing_message::ConnectionId;
use crate::outgoing_message::OutgoingEnvelope;
use crate::outgoing_message::OutgoingMessage;
use crate::outgoing_message::OutgoingMessageSender;
use crate::transport::AppServerTransport;
use anyhow::Result;
use app_test_support::write_mock_responses_config_toml;
use codex_analytics::AppServerRpcTransport;
use codex_app_server_protocol::ClientInfo;
use codex_app_server_protocol::ClientRequest;
use codex_app_server_protocol::ConfigBatchWriteParams;
use codex_app_server_protocol::ConfigEdit;
use codex_app_server_protocol::ConfigWriteResponse;
use codex_app_server_protocol::InitializeCapabilities;
use codex_app_server_protocol::InitializeParams;
use codex_app_server_protocol::InitializeResponse;
use codex_app_server_protocol::MergeStrategy;
use codex_app_server_protocol::ModelListParams;
use codex_app_server_protocol::ModelListResponse;
use codex_app_server_protocol::RequestId;
use codex_app_server_protocol::ThreadStartParams;
use codex_app_server_protocol::ThreadStartResponse;
use codex_arg0::Arg0DispatchPaths;
use codex_core::config::Config;
use codex_core::config::ConfigBuilder;
use codex_core::config_loader::CloudRequirementsLoader;
use codex_core::config_loader::LoaderOverrides;
use codex_exec_server::EnvironmentManager;
use codex_feedback::CodexFeedback;
use codex_login::AuthManager;
use codex_protocol::protocol::SessionSource;
use serde_json::json;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::sync::mpsc;
use wiremock::Mock;
use wiremock::MockServer;
use wiremock::ResponseTemplate;
use wiremock::matchers::method;
use wiremock::matchers::path;

const TEST_CONNECTION_ID: ConnectionId = ConnectionId(19);

struct ModelListHarness {
    server: MockServer,
    _codex_home: TempDir,
    processor: Arc<MessageProcessor>,
    outgoing_rx: mpsc::Receiver<OutgoingEnvelope>,
    session: Arc<ConnectionSessionState>,
}

impl ModelListHarness {
    async fn new() -> Result<Self> {
        let server = MockServer::start().await;
        let codex_home = TempDir::new()?;
        let config = Arc::new(build_test_config(codex_home.path(), &server.uri()).await?);
        let (processor, outgoing_rx) = build_test_processor(config);
        let mut harness = Self {
            server,
            _codex_home: codex_home,
            processor,
            outgoing_rx,
            session: Arc::new(ConnectionSessionState::default()),
        };

        let _: InitializeResponse = harness
            .request(ClientRequest::Initialize {
                request_id: RequestId::Integer(1),
                params: InitializeParams {
                    client_info: ClientInfo {
                        name: "codex-app-server-model-list-tests".to_string(),
                        title: None,
                        version: "0.1.0".to_string(),
                    },
                    capabilities: Some(InitializeCapabilities {
                        experimental_api: true,
                        ..Default::default()
                    }),
                },
            })
            .await;
        assert!(harness.session.initialized());
        Ok(harness)
    }

    async fn request<T>(&mut self, request: ClientRequest) -> T
    where
        T: serde::de::DeserializeOwned,
    {
        let request_id = match request.id() {
            RequestId::Integer(request_id) => *request_id,
            request_id => panic!("expected integer request id in test harness, got {request_id:?}"),
        };

        let request =
            serde_json::from_value(serde_json::to_value(request).expect("serialize request"))
                .expect("request should convert to JSON-RPC");

        self.processor
            .process_request(
                TEST_CONNECTION_ID,
                request,
                AppServerTransport::Stdio,
                Arc::clone(&self.session),
            )
            .await;
        read_response(&mut self.outgoing_rx, request_id).await
    }

    async fn shutdown(self) {
        self.processor.shutdown_threads().await;
        self.processor.drain_background_tasks().await;
    }
}

async fn build_test_config(codex_home: &Path, server_uri: &str) -> Result<Config> {
    write_mock_responses_config_toml(
        codex_home,
        server_uri,
        &BTreeMap::new(),
        /*auto_compact_limit*/ 8_192,
        Some(false),
        "mock_provider",
        "compact",
    )?;

    Ok(ConfigBuilder::default()
        .codex_home(codex_home.to_path_buf())
        .build()
        .await?)
}

fn build_test_processor(
    config: Arc<Config>,
) -> (Arc<MessageProcessor>, mpsc::Receiver<OutgoingEnvelope>) {
    let (outgoing_tx, outgoing_rx) = mpsc::channel(16);
    let outgoing = Arc::new(OutgoingMessageSender::new(outgoing_tx));
    let processor = Arc::new(MessageProcessor::new(MessageProcessorArgs {
        outgoing,
        arg0_paths: Arg0DispatchPaths::default(),
        config: config.clone(),
        environment_manager: Arc::new(EnvironmentManager::new(/*exec_server_url*/ None)),
        cli_overrides: Vec::new(),
        loader_overrides: LoaderOverrides::default(),
        cloud_requirements: CloudRequirementsLoader::default(),
        feedback: CodexFeedback::new(),
        log_db: None,
        config_warnings: Vec::new(),
        session_source: SessionSource::VSCode,
        auth_manager: AuthManager::shared_from_config(
            config.as_ref(),
            /*enable_codex_api_key_env*/ false,
        ),
        rpc_transport: AppServerRpcTransport::Stdio,
        remote_control_handle: None,
    }));
    (processor, outgoing_rx)
}

async fn read_response<T: serde::de::DeserializeOwned>(
    outgoing_rx: &mut mpsc::Receiver<OutgoingEnvelope>,
    request_id: i64,
) -> T {
    loop {
        let envelope = tokio::time::timeout(std::time::Duration::from_secs(5), outgoing_rx.recv())
            .await
            .expect("timed out waiting for response")
            .expect("outgoing channel closed");
        let OutgoingEnvelope::ToConnection {
            connection_id,
            message,
            ..
        } = envelope
        else {
            continue;
        };
        if connection_id != TEST_CONNECTION_ID {
            continue;
        }
        match message {
            OutgoingMessage::Response(response) => {
                if response.id != RequestId::Integer(request_id) {
                    continue;
                }
                return serde_json::from_value(response.result)
                    .expect("response payload should deserialize");
            }
            OutgoingMessage::Error(error) => {
                if error.id != RequestId::Integer(request_id) {
                    continue;
                }
                panic!(
                    "request {request_id} failed unexpectedly: code={} message={}",
                    error.error.code, error.error.message
                );
            }
            OutgoingMessage::Request(_) | OutgoingMessage::AppServerNotification(_) => continue,
        }
    }
}

fn set_config_value(key_path: &str, value: serde_json::Value) -> ConfigEdit {
    ConfigEdit {
        key_path: key_path.to_string(),
        value,
        merge_strategy: MergeStrategy::Replace,
    }
}

#[tokio::test(flavor = "current_thread")]
async fn model_list_uses_provider_added_via_config_batch_write_immediately() -> Result<()> {
    let mut harness = ModelListHarness::new().await?;

    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "list",
            "data": [
                {
                    "id": "llama-3.3-70b-versatile",
                    "object": "model",
                    "context_window": 128000,
                    "supported_parameters": ["tools"]
                }
            ]
        })))
        .mount(&harness.server)
        .await;

    let _: ConfigWriteResponse = harness
        .request(ClientRequest::ConfigBatchWrite {
            request_id: RequestId::Integer(2),
            params: ConfigBatchWriteParams {
                edits: vec![
                    set_config_value("model_provider", json!("groq")),
                    set_config_value("model_providers.groq.name", json!("Groq")),
                    set_config_value("model_providers.groq.base_url", json!(harness.server.uri())),
                    set_config_value("model_providers.groq.wire_api", json!("chat")),
                    set_config_value("model_providers.groq.requires_openai_auth", json!(false)),
                    set_config_value("model_providers.groq.supports_websockets", json!(false)),
                    set_config_value(
                        "model_providers.groq.experimental_bearer_token",
                        json!("Test API Key"),
                    ),
                ],
                file_path: None,
                expected_version: None,
                reload_user_config: true,
            },
        })
        .await;

    let provider_specific: ModelListResponse = harness
        .request(ClientRequest::ModelList {
            request_id: RequestId::Integer(3),
            params: ModelListParams {
                cursor: None,
                limit: None,
                include_hidden: Some(true),
                model_provider: Some("groq".to_string()),
            },
        })
        .await;
    let provider_requests = harness.server.received_requests().await.unwrap_or_default();
    assert!(
        !provider_specific.data.is_empty(),
        "expected newly added provider to return at least one model; requests={provider_requests:#?}"
    );

    let active_provider: ModelListResponse = harness
        .request(ClientRequest::ModelList {
            request_id: RequestId::Integer(4),
            params: ModelListParams {
                cursor: None,
                limit: None,
                include_hidden: Some(true),
                model_provider: None,
            },
        })
        .await;
    let active_requests = harness.server.received_requests().await.unwrap_or_default();
    assert!(
        !active_provider.data.is_empty(),
        "expected active provider model list to be non-empty after config write; requests={active_requests:#?}"
    );
    assert_eq!(provider_specific.data, active_provider.data);

    harness.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn thread_start_uses_provider_added_via_config_batch_write_immediately() -> Result<()> {
    let mut harness = ModelListHarness::new().await?;

    let model = "llama-3.3-70b-versatile";
    let _: ConfigWriteResponse = harness
        .request(ClientRequest::ConfigBatchWrite {
            request_id: RequestId::Integer(10),
            params: ConfigBatchWriteParams {
                edits: vec![
                    set_config_value("model_provider", json!("groq")),
                    set_config_value("model", json!(model)),
                    set_config_value("model_providers.groq.name", json!("Groq")),
                    set_config_value("model_providers.groq.base_url", json!(harness.server.uri())),
                    set_config_value("model_providers.groq.wire_api", json!("chat")),
                    set_config_value("model_providers.groq.requires_openai_auth", json!(false)),
                    set_config_value("model_providers.groq.supports_websockets", json!(false)),
                    set_config_value(
                        "model_providers.groq.experimental_bearer_token",
                        json!("Test API Key"),
                    ),
                ],
                file_path: None,
                expected_version: None,
                reload_user_config: true,
            },
        })
        .await;

    let started: ThreadStartResponse = harness
        .request(ClientRequest::ThreadStart {
            request_id: RequestId::Integer(11),
            params: ThreadStartParams::default(),
        })
        .await;

    assert_eq!(started.model_provider, "groq");
    assert_eq!(started.model, model);

    harness.shutdown().await;
    Ok(())
}
