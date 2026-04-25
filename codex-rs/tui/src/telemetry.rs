#[allow(unused_imports)]
#[cfg(feature = "telemetry")]
pub(crate) use codex_otel::RuntimeMetricTotals;
#[cfg(feature = "telemetry")]
pub(crate) use codex_otel::RuntimeMetricsSummary;
#[cfg(feature = "telemetry")]
pub(crate) use codex_otel::SessionTelemetry;
#[cfg(feature = "telemetry")]
pub(crate) use codex_otel::TelemetryAuthMode;

#[cfg(not(feature = "telemetry"))]
use codex_protocol::ThreadId;
#[cfg(not(feature = "telemetry"))]
use codex_protocol::openai_models::ReasoningEffort;
#[cfg(not(feature = "telemetry"))]
use codex_protocol::protocol::AskForApproval;
#[cfg(not(feature = "telemetry"))]
use codex_protocol::protocol::ReviewDecision;
#[cfg(not(feature = "telemetry"))]
use codex_protocol::protocol::SandboxPolicy;
#[cfg(not(feature = "telemetry"))]
use codex_protocol::protocol::SessionSource;
#[cfg(not(feature = "telemetry"))]
use std::time::Duration;

#[cfg(not(feature = "telemetry"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TelemetryAuthMode {
    ApiKey,
    Chatgpt,
}

#[cfg(not(feature = "telemetry"))]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct RuntimeMetricTotals {
    pub count: u64,
    pub duration_ms: u64,
}

#[cfg(not(feature = "telemetry"))]
impl RuntimeMetricTotals {
    pub(crate) fn is_empty(self) -> bool {
        self.count == 0 && self.duration_ms == 0
    }

    pub(crate) fn merge(&mut self, other: Self) {
        self.count = self.count.saturating_add(other.count);
        self.duration_ms = self.duration_ms.saturating_add(other.duration_ms);
    }
}

#[cfg(not(feature = "telemetry"))]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct RuntimeMetricsSummary {
    pub tool_calls: RuntimeMetricTotals,
    pub api_calls: RuntimeMetricTotals,
    pub streaming_events: RuntimeMetricTotals,
    pub websocket_calls: RuntimeMetricTotals,
    pub websocket_events: RuntimeMetricTotals,
    pub responses_api_overhead_ms: u64,
    pub responses_api_inference_time_ms: u64,
    pub responses_api_engine_iapi_ttft_ms: u64,
    pub responses_api_engine_service_ttft_ms: u64,
    pub responses_api_engine_iapi_tbt_ms: u64,
    pub responses_api_engine_service_tbt_ms: u64,
    pub turn_ttft_ms: u64,
    pub turn_ttfm_ms: u64,
}

#[cfg(not(feature = "telemetry"))]
impl RuntimeMetricsSummary {
    pub(crate) fn is_empty(self) -> bool {
        self.tool_calls.is_empty()
            && self.api_calls.is_empty()
            && self.streaming_events.is_empty()
            && self.websocket_calls.is_empty()
            && self.websocket_events.is_empty()
            && self.responses_api_overhead_ms == 0
            && self.responses_api_inference_time_ms == 0
            && self.responses_api_engine_iapi_ttft_ms == 0
            && self.responses_api_engine_service_ttft_ms == 0
            && self.responses_api_engine_iapi_tbt_ms == 0
            && self.responses_api_engine_service_tbt_ms == 0
            && self.turn_ttft_ms == 0
            && self.turn_ttfm_ms == 0
    }

    pub(crate) fn merge(&mut self, other: Self) {
        self.tool_calls.merge(other.tool_calls);
        self.api_calls.merge(other.api_calls);
        self.streaming_events.merge(other.streaming_events);
        self.websocket_calls.merge(other.websocket_calls);
        self.websocket_events.merge(other.websocket_events);
        if other.responses_api_overhead_ms > 0 {
            self.responses_api_overhead_ms = other.responses_api_overhead_ms;
        }
        if other.responses_api_inference_time_ms > 0 {
            self.responses_api_inference_time_ms = other.responses_api_inference_time_ms;
        }
        if other.responses_api_engine_iapi_ttft_ms > 0 {
            self.responses_api_engine_iapi_ttft_ms = other.responses_api_engine_iapi_ttft_ms;
        }
        if other.responses_api_engine_service_ttft_ms > 0 {
            self.responses_api_engine_service_ttft_ms = other.responses_api_engine_service_ttft_ms;
        }
        if other.responses_api_engine_iapi_tbt_ms > 0 {
            self.responses_api_engine_iapi_tbt_ms = other.responses_api_engine_iapi_tbt_ms;
        }
        if other.responses_api_engine_service_tbt_ms > 0 {
            self.responses_api_engine_service_tbt_ms = other.responses_api_engine_service_tbt_ms;
        }
        if other.turn_ttft_ms > 0 {
            self.turn_ttft_ms = other.turn_ttft_ms;
        }
        if other.turn_ttfm_ms > 0 {
            self.turn_ttfm_ms = other.turn_ttfm_ms;
        }
    }

    pub(crate) fn responses_api_summary(&self) -> Self {
        Self {
            responses_api_overhead_ms: self.responses_api_overhead_ms,
            responses_api_inference_time_ms: self.responses_api_inference_time_ms,
            responses_api_engine_iapi_ttft_ms: self.responses_api_engine_iapi_ttft_ms,
            responses_api_engine_service_ttft_ms: self.responses_api_engine_service_ttft_ms,
            responses_api_engine_iapi_tbt_ms: self.responses_api_engine_iapi_tbt_ms,
            responses_api_engine_service_tbt_ms: self.responses_api_engine_service_tbt_ms,
            ..Self::default()
        }
    }
}

#[cfg(not(feature = "telemetry"))]
#[derive(Clone, Default)]
pub(crate) struct SessionTelemetry;

#[cfg(not(feature = "telemetry"))]
impl SessionTelemetry {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        _conversation_id: ThreadId,
        _model: &str,
        _slug: &str,
        _account_id: Option<String>,
        _account_email: Option<String>,
        _auth_mode: Option<TelemetryAuthMode>,
        _originator: String,
        _log_user_prompts: bool,
        _terminal_type: String,
        _session_source: SessionSource,
    ) -> Self {
        Self
    }

    pub(crate) fn counter(&self, _name: &str, _inc: i64, _tags: &[(&str, &str)]) {}

    pub(crate) fn record_duration(&self, _name: &str, _duration: Duration, _tags: &[(&str, &str)]) {
    }

    pub(crate) fn reset_runtime_metrics(&self) {}

    pub(crate) fn runtime_metrics_summary(&self) -> Option<RuntimeMetricsSummary> {
        None
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn conversation_starts(
        &self,
        _provider_name: &str,
        _reasoning_effort: Option<ReasoningEffort>,
        _reasoning_summary: codex_protocol::config_types::ReasoningSummary,
        _context_window: Option<i64>,
        _auto_compact_token_limit: Option<i64>,
        _approval_policy: AskForApproval,
        _sandbox_policy: SandboxPolicy,
        _mcp_servers: Vec<&str>,
        _active_profile: Option<String>,
    ) {
    }

    pub(crate) fn review_finished(
        &self,
        _target: &str,
        _decision: ReviewDecision,
        _source: Option<&str>,
    ) {
    }
}
