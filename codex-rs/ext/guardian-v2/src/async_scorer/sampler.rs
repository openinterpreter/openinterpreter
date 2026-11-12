//! Builds tool-less risk requests and publishes the first classifier output.
//! Both transports share request identity, retry, cancellation, and output handling.

mod connection_pool;
mod execution;

pub(super) use telemetry::SampledScore;

use super::request::PreparedRequest;
use super::score::PendingScore;
use codex_login::AuthOwner;
use connection_pool::ConnectionPool;
use std::collections::VecDeque;
use std::sync::Arc;

use codex_api::ApiError;
use codex_api::Reasoning;
use codex_api::ReasoningContext;
use codex_api::ResponsesApiRequest;
use codex_context_fragments::RenderedFragment;
use codex_extension_api::ExtensionMetrics;
use codex_history::ResponseItemEnvelope;
use codex_http_client::HttpClientFactory;
use codex_login::AgentIdentityAuthPolicy;
use codex_model_provider::SharedModelProvider;
use codex_model_provider::WorkspaceRoutingContext;
use codex_protocol::error::CodexErr;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::openai_models::ReasoningEffort;
use codex_protocol::protocol::SessionSource;
use thiserror::Error;
use tokio::sync::oneshot;
use uuid::Uuid;

pub(crate) const MODEL: &str = "gpt-5.6-luna";
pub(crate) const CLASSIFICATION_TOKEN_USAGE_METRIC: &str =
    "codex.guardian_v2.classification.token_usage";
const MAX_OUTPUT_BYTES: usize = 8 * 1024;
pub(super) const INITIAL_WEBSOCKET_CONNECTIONS: usize = if cfg!(test) { 2 } else { 8 };
const MAX_CONCURRENT_REQUESTS: usize = 16;

/// Host-owned provider, authentication, and attribution for one Luna connection.
pub struct LunaSamplerConfig {
    /// Provider and credentials selected for the owning thread.
    pub provider: SharedModelProvider,
    /// Provider and feature policy shared with ordinary model requests.
    pub api_key_cyber_access_programs: ApiKeyCyberAccessPrograms,
    /// False when the parent host confines its explicit program to that thread.
    pub inherit_parent_cyber_access_program: bool,
    /// Routing scope and retained configuration layers for the owning thread.
    pub workspace_routing: WorkspaceRoutingContext,
    /// Effective proxy, custom-CA, and cookie configuration.
    pub http_client_factory: HttpClientFactory,
    /// Agent-identity policy selected for the owning thread.
    pub agent_identity_policy: AgentIdentityAuthPolicy,
    /// Host-resolved source used to scope agent-identity authentication.
    pub session_source: SessionSource,
    /// Owning runtime session identifier.
    pub session_id: String,
    /// Owning thread identifier.
    pub thread_id: String,
    /// Optional host-resolved request originator.
    pub originator: Option<String>,
    /// Optional inference service tier.
    pub service_tier: Option<String>,
    /// Luna model's host-resolved encrypted-compaction compatibility hash.
    pub luna_compaction_hash: Option<String>,
    /// Complete input allowance resolved for the classifier model.
    pub max_input_tokens: usize,
    /// Host-provided metrics capability with the owning session's attribution.
    pub metrics: Option<Arc<dyn ExtensionMetrics>>,
}

/// One tool-less Luna classification request.
pub struct LunaSamplingRequest {
    /// Generator selected by the owning session for response items.
    pub item_id_generator: codex_protocol::ItemIdGenerator,
    /// Cyber selection captured from the owning turn; Daybreak reviews use Blue.
    pub cyber_access_program: Option<CyberAccessProgram>,
    /// ID of the response handling the classified tool.
    pub parent_response_id: Option<String>,
    /// Trusted classifier instructions with their role and content attribution.
    pub instructions: RenderedFragment,
    /// Composed evidence messages, with roles, annotations and content order intact.
    pub input: Vec<ResponseItem>,
    /// Opaque parent compaction to reuse only for compatible model configurations.
    pub parent_compaction: Option<ResponseItem>,
    /// Reasoning budget explicitly selected for this request.
    pub reasoning_effort: ReasoningEffort,
    /// Owning turn that initiated this classification, not the classifier turn.
    pub parent_turn_id: String,
    /// Trusted causal root of the owning turn, absent when unknown or ambiguous.
    pub root_turn_id: Option<String>,
    /// Sampling expiry remains active until its response finishes draining.
    pub(super) pending_score: Option<PendingScore>,
}

/// Admitted retained input and Luna settings consumed together at execution.
pub(super) struct ConversationSample {
    pub(super) input: PreparedRequest,
    pub(super) sampling: LunaSamplingRequest,
}

/// Failures returned while connecting or sampling the Luna model.
#[derive(Debug, Error)]
pub enum LunaSamplerError {
    /// The thread's provider or scoped credentials could not be resolved.
    #[error("could not resolve the Luna model provider: {0}")]
    Provider(#[source] CodexErr),
    /// The Responses request could not be opened or streamed.
    #[error("Luna Responses request failed: {0}")]
    Api(#[source] ApiError),
    /// The provider's WebSocket connect deadline elapsed.
    #[error("Luna Responses WebSocket connection timed out")]
    ConnectionTimeout,
    /// The response did not contain an assistant text value.
    #[error("Luna response did not contain assistant output")]
    MissingOutput,
    /// The response exceeded the bounded output limit.
    #[error("Luna response exceeded the output limit")]
    OutputTooLarge,
    /// The classification expired, lost its owning generation, or was replaced at capacity.
    #[error("Luna request was superseded by a newer classification")]
    Superseded,
    /// The supplied parent checkpoint cannot be consumed by this Luna configuration.
    #[error("parent compaction is incompatible with Luna")]
    IncompatibleCompaction,
    /// The complete classifier input exceeded the model allowance.
    #[error("Luna input exceeds the complete request budget")]
    InputTooLarge,
}

struct ActiveRequest {
    supersede: oneshot::Sender<()>,
    scored: Arc<AtomicBool>,
}

/// Runs bounded Luna classifications over pooled WebSockets or HTTP.
pub struct LunaSampler {
    config: Arc<LunaSamplerConfig>,
    connections: Arc<ConnectionPool>,
}

impl LunaSampler {
    /// A checkpoint is reusable only when both models declare the same nonempty hash.
    pub(super) fn supports_parent_compaction(&self, parent_hash: Option<&str>) -> bool {
        parent_hash
            .zip(self.config.luna_compaction_hash.as_deref())
            .is_some_and(|(parent_hash, luna_hash)| {
                !parent_hash.is_empty() && parent_hash == luna_hash
            })
    }

    pub(super) fn new(config: LunaSamplerConfig) -> Self {
        let config = Arc::new(config);
        Self {
            connections: ConnectionPool::new(Arc::clone(&config)),
            config,
        }
    }

    pub(super) async fn prewarm(&self) {
        self.connections.prewarm().await;
    }

    fn auth_owner(&self) -> Option<AuthOwner> {
        let provider = &self.config.provider;
        if provider.info().auth.is_none() {
            codex_model_provider::request_auth_owner(provider.as_ref())
        } else {
            None
        }
    }

    /// Sends one tool-less classification request using an available transport.
    pub async fn sample(&self, request: LunaSamplingRequest) -> Result<String, LunaSamplerError> {
        if request.parent_compaction.is_some()
            && !self.supports_parent_compaction(request.parent_compaction_hash.as_deref())
        {
            return Err(LunaSamplerError::IncompatibleCompaction);
        }
        // A classification is its own inference turn; retries keep that identity.
        let turn_id = Uuid::now_v7().to_string();
        let parent_response_id = request.parent_response_id;
        let parent_turn_id = request.parent_turn_id;
        let root_turn_id = request.root_turn_id;
        let mut input = vec![
            ResponseItem::AdditionalTools {
                id: None,
                role: "developer".to_owned(),
                tools: Vec::new(),
            },
            ResponseItem::from(request.instructions),
        ];
        if let Some(parent_compaction) = request.parent_compaction {
            input.push(parent_compaction);
        }
        let mut evidence = request.input;
        for item in &mut evidence {
            if let ResponseItem::Message { content, .. } = item {
                for content in content {
                    if let ContentItem::InputImage { detail, .. } = content {
                        *detail = None;
                    }
                }
            }
        }
        input.extend(evidence);
        // Assign IDs once so retries reuse the same input item identities.
        for item in &mut input {
            if item.id().is_none()
                && let Some(prefix) = item.id_prefix()
            {
                item.set_id(Some((request.item_id_generator)(prefix)));
            }
        }
        let total_tokens = input
            .iter()
            .map(codex_guardian_context::estimate_input_tokens)
            .fold(0usize, usize::saturating_add);
        if let Some(metrics) = self.config.metrics.as_deref() {
            for (component, tokens) in [
                ("existing_context", 0),
                ("new_input", total_tokens),
                ("total", total_tokens),
            ] {
                metrics.histogram_with_boundaries(
                    codex_guardian_context::REQUEST_TOKENS_METRIC,
                    i64::try_from(tokens).unwrap_or(i64::MAX),
                    codex_guardian_context::REQUEST_TOKENS_BOUNDARIES,
                    &[("target", "async"), ("component", component)],
                );
            }
        }
        // Oversized classifications defer to sync with the existing failure score.
        if total_tokens > self.config.max_input_tokens.saturating_sub(/*rhs*/ 256) {
            return Err(LunaSamplerError::InputTooLarge);
        }
        let request = ResponsesApiRequest {
            model: MODEL.to_owned(),
            input,
            tools: None,
            tool_choice: "none".to_owned(),
            parallel_tool_calls: false,
            reasoning: Some(Reasoning {
                effort: Some(request.reasoning_effort.clone()),
                summary: None,
                context: Some(ReasoningContext::AllTurns),
            }),
            store: false,
            stream: true,
            stream_options: None,
            include: Vec::new(),
            service_tier: None,
            prompt_cache_key: Some(format!("guardian-v2:{}", self.config.thread_id)),
            prompt_cache_options: None,
            text: None,
            client_metadata: None,
            access_programs: None,
        };
        let (supersede, superseded) = oneshot::channel();
        let scored = Arc::new(AtomicBool::new(false));
        {
            let mut active_requests = self
                .active_requests
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            active_requests.retain(|request| !request.supersede.is_closed());
            if active_requests.len() == MAX_CONCURRENT_REQUESTS {
                let oldest_scored = active_requests
                    .iter()
                    .position(|request| request.scored.load(Ordering::Relaxed))
                    .unwrap_or(0);
                if let Some(oldest) = active_requests.remove(oldest_scored) {
                    let _ = oldest.supersede.send(());
                }
            }
            active_requests.push_back(ActiveRequest {
                supersede,
                scored: Arc::clone(&scored),
            });
        }
        execution::SamplingExecution {
            config: Arc::clone(&self.config),
            connections: Arc::clone(&self.connections),
            request,
            turn_id,
            parent_response_id,
            parent_turn_id,
            root_turn_id,
        }
        .run(superseded, scored)
        .await
    }
}

#[cfg(test)]
#[path = "sampler_tests.rs"]
pub(super) mod tests;
