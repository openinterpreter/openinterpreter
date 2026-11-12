//! Request-scoped settings and capabilities, including the durable context snapshot.

use std::sync::Arc;

use crate::agents_md::LoadedAgentsMd;
use crate::config::TokenBudgetConfig;
use crate::environment_selection::TurnEnvironmentSnapshot;
use crate::realtime_conversation::RealtimeConversationSnapshot;
use crate::session::step_settings::ResolvedStepSettings;
use crate::session::turn_context::TurnContext;
use crate::session::turn_context::TurnEnvironment;
use crate::tools::router::ToolRouter;
use codex_extension_api::ExtensionData;
use codex_features::Feature;
use codex_file_system::EnvironmentAccess;
use codex_mcp::McpBinding;
use codex_model_provider_info::AMAZON_BEDROCK_PROVIDER_ID;
use codex_model_provider_info::AMAZON_BEDROCK_RUNTIME_PROVIDER_ID;
use codex_model_provider_info::GCP_PROVIDER_ID;
use codex_otel::SessionTelemetry;
use codex_protocol::items::ModelInvocationContext;
use codex_protocol::protocol::TurnContextItem;

/// Inputs for the next step, published together so capture cannot mix versions.
#[derive(Debug)]
pub(crate) struct StepInputs {
    pub(crate) settings: Arc<ResolvedStepSettings>,
    /// Selection is fixed within this version; attachment startup may still finish.
    pub(crate) environments: TurnEnvironmentSnapshot,
}

/// Request-scoped state that may change between model sampling requests.
#[derive(Clone)]
pub(crate) struct StepContext {
    /// Input prefix captured for this normal inference attempt, acknowledged with its output.
    pub(crate) sampled_through_sequence: Option<u64>,
    /// Final serving value for this invocation; retained by its code cell across waits.
    pub(crate) serving_assessment: Option<Arc<crate::guardian::ServingAssessment>>,
    pub(crate) turn: Arc<TurnContext>,
    /// Preempts this request and yields its code-mode observations when user input arrives.
    pub(crate) preempt: Option<CancellationToken>,
    /// Realtime call activity and instructions captured for this sampling request.
    pub(crate) realtime: RealtimeConversationSnapshot,
    /// One immutable settings version captured before request preparation.
    pub(crate) settings: Arc<ResolvedStepSettings>,
    /// Frozen turn preferences resolved against this step's captured model.
    pub(crate) token_budget: Option<TokenBudgetConfig>,
    /// Telemetry context tagged with this sampling request's model.
    pub(crate) session_telemetry: SessionTelemetry,
    pub(crate) environments: TurnEnvironmentSnapshot,
    /// Child bindings captured with this step; configured-only when opted in.
    pub(crate) inherited_environments: TurnEnvironmentSnapshot,
    /// Admitted executor hooks bound to this step's discovery, plugin policy, and MCP routing.
    pub(crate) executor_hook_sources: Vec<ExecutorPluginHookSource>,
    /// Keeps the extension inputs used to build this step's tools for its World State as well.
    pub(crate) extension_data: Arc<ExtensionData>,
    /// The exact MCP connections, configuration, and catalog captured for this step.
    pub(crate) mcp: Arc<McpBinding>,
    /// The finalized tool plan advertised and executed for this exact sampling request.
    pub(crate) tool_router: Arc<ToolRouter>,
    /// The canonical AGENTS.md value observed with this environment snapshot.
    pub(crate) loaded_agents_md: Option<Arc<LoadedAgentsMd>>,
}

impl StepContext {
    /// Persist the summary captured for this request, even after a live settings update.
    pub(crate) fn to_turn_context_item(&self) -> TurnContextItem {
        let mut item = self.turn.to_turn_context_item();
        item.summary = self.settings.reasoning_summary;
        item
    }

    pub(crate) fn model_context(&self) -> ModelInvocationContext {
        ModelInvocationContext {
            model_slug: self.settings.model_info.slug.clone(),
            reasoning_effort: self
                .settings
                .effective_reasoning_effort()
                .map(|effort| effort.to_string()),
        }
    }
}
