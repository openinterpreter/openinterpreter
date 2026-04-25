use crate::auth::SharedAuthProvider;
use crate::endpoint::session::EndpointSession;
use crate::error::ApiError;
use crate::model_compatibility_catalog::CompatibleModelCatalogEntry;
use crate::model_compatibility_catalog::compatible_model_catalog_entry;
use crate::provider::Provider;
use crate::provider::is_anthropic_provider;
use codex_client::HttpTransport;
use codex_client::RequestTelemetry;
use codex_protocol::config_types::ReasoningSummary;
use codex_protocol::openai_models::ConfigShellToolType;
use codex_protocol::openai_models::InputModality;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::openai_models::ModelVisibility;
use codex_protocol::openai_models::ModelsResponse;
use codex_protocol::openai_models::ReasoningEffort;
use codex_protocol::openai_models::ReasoningEffortPreset;
use codex_protocol::openai_models::TruncationPolicyConfig;
use codex_protocol::openai_models::WebSearchToolType;
use http::HeaderMap;
use http::Method;
use http::header::ETAG;
use std::sync::Arc;

pub struct ModelsClient<T: HttpTransport> {
    session: EndpointSession<T>,
}

#[derive(serde::Deserialize)]
struct OpenAiCompatibleModelsResponse {
    data: Vec<OpenAiCompatibleModel>,
}

#[derive(serde::Deserialize)]
struct AnthropicModelsResponse {
    data: Vec<AnthropicModel>,
}

#[derive(serde::Deserialize)]
struct AnthropicModel {
    id: String,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    max_input_tokens: Option<i64>,
    capabilities: AnthropicModelCapabilities,
}

#[derive(Default, serde::Deserialize)]
struct AnthropicModelCapabilities {
    #[serde(default)]
    effort: AnthropicEffortCapabilities,
    #[serde(default)]
    image_input: AnthropicCapabilitySupport,
    #[serde(default)]
    pdf_input: AnthropicCapabilitySupport,
    #[serde(default)]
    structured_outputs: AnthropicCapabilitySupport,
    #[serde(default)]
    thinking: AnthropicThinkingCapabilities,
}

#[derive(Default, serde::Deserialize)]
struct AnthropicCapabilitySupport {
    #[serde(default)]
    supported: bool,
}

#[derive(Default, serde::Deserialize)]
struct AnthropicEffortCapabilities {
    #[serde(default)]
    supported: bool,
    #[serde(default)]
    low: AnthropicCapabilitySupport,
    #[serde(default)]
    medium: AnthropicCapabilitySupport,
    #[serde(default)]
    high: AnthropicCapabilitySupport,
    #[serde(default, rename = "max")]
    max_effort: AnthropicCapabilitySupport,
}

#[derive(Default, serde::Deserialize)]
struct AnthropicThinkingCapabilities {
    #[serde(default)]
    supported: bool,
    #[serde(default)]
    types: AnthropicThinkingTypes,
}

#[derive(Default, serde::Deserialize)]
struct AnthropicThinkingTypes {
    #[serde(default)]
    enabled: AnthropicCapabilitySupport,
    #[serde(default)]
    adaptive: AnthropicCapabilitySupport,
}

#[derive(serde::Deserialize)]
struct OpenAiCompatibleModel {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    context_window: Option<i64>,
    #[serde(default)]
    context_length: Option<i64>,
    #[serde(default)]
    supported_parameters: Vec<String>,
    #[serde(default)]
    architecture: Option<OpenAiCompatibleModelArchitecture>,
    #[serde(default)]
    top_provider: Option<OpenAiCompatibleTopProvider>,
}

#[derive(serde::Deserialize)]
struct OpenAiCompatibleModelArchitecture {
    #[serde(default)]
    input_modalities: Vec<String>,
}

#[derive(serde::Deserialize)]
struct OpenAiCompatibleTopProvider {
    #[serde(default)]
    context_length: Option<i64>,
}

impl<T: HttpTransport> ModelsClient<T> {
    pub fn new(transport: T, provider: Provider, auth: SharedAuthProvider) -> Self {
        Self {
            session: EndpointSession::new(transport, provider, auth),
        }
    }

    pub fn with_telemetry(self, request: Option<Arc<dyn RequestTelemetry>>) -> Self {
        Self {
            session: self.session.with_request_telemetry(request),
        }
    }

    fn path(provider: &Provider) -> &'static str {
        if is_anthropic_provider(provider.name.as_str(), Some(provider.base_url.as_str()))
            && matches!(url::Url::parse(provider.base_url.as_str()), Ok(url) if url.path() == "/" || url.path().is_empty())
        {
            "v1/models"
        } else {
            "models"
        }
    }

    fn append_client_version_query(req: &mut codex_client::Request, client_version: &str) {
        let separator = if req.url.contains('?') { '&' } else { '?' };
        req.url = format!("{}{}client_version={client_version}", req.url, separator);
    }

    pub async fn list_models(
        &self,
        client_version: &str,
        extra_headers: HeaderMap,
    ) -> Result<(Vec<ModelInfo>, Option<String>), ApiError> {
        let resp = self
            .session
            .execute_with(
                Method::GET,
                Self::path(self.session.provider()),
                extra_headers,
                /*body*/ None,
                |req| {
                    Self::append_client_version_query(req, client_version);
                },
            )
            .await?;

        let header_etag = resp
            .headers
            .get(ETAG)
            .and_then(|value| value.to_str().ok())
            .map(ToString::to_string);

        let models = decode_models_response(&resp.body)?;

        Ok((models, header_etag))
    }
}

fn decode_models_response(body: &[u8]) -> Result<Vec<ModelInfo>, ApiError> {
    if let Ok(ModelsResponse { models }) = serde_json::from_slice::<ModelsResponse>(body) {
        return Ok(models);
    }

    if let Ok(AnthropicModelsResponse { data }) =
        serde_json::from_slice::<AnthropicModelsResponse>(body)
    {
        return Ok(data
            .into_iter()
            .map(anthropic_model_info_from_capabilities)
            .collect());
    }

    if let Ok(OpenAiCompatibleModelsResponse { data }) =
        serde_json::from_slice::<OpenAiCompatibleModelsResponse>(body)
    {
        return Ok(data
            .into_iter()
            .map(openai_compatible_model_info_from_id)
            .collect());
    }

    Err(ApiError::Stream(format!(
        "failed to decode models response; body: {}",
        String::from_utf8_lossy(body)
    )))
}

fn anthropic_model_info_from_capabilities(model: AnthropicModel) -> ModelInfo {
    let supported_reasoning_levels =
        anthropic_supported_reasoning_levels(&model.capabilities.effort);
    let supports_reasoning = model.capabilities.thinking.supported
        || model.capabilities.thinking.types.enabled.supported
        || model.capabilities.thinking.types.adaptive.supported;
    let default_reasoning_level = if supports_reasoning {
        Some(ReasoningEffort::Medium)
    } else {
        None
    };
    let mut input_modalities = vec![InputModality::Text];
    if model.capabilities.image_input.supported {
        input_modalities.push(InputModality::Image);
    }

    ModelInfo {
        slug: model.id.clone(),
        display_name: model.display_name.unwrap_or(model.id),
        description: anthropic_description(&model.capabilities),
        default_reasoning_level,
        supported_reasoning_levels,
        shell_type: ConfigShellToolType::Default,
        visibility: ModelVisibility::List,
        supported_in_api: true,
        priority: 0,
        availability_nux: None,
        upgrade: None,
        base_instructions: String::new(),
        model_messages: None,
        supports_reasoning_summaries: false,
        default_reasoning_summary: ReasoningSummary::Auto,
        support_verbosity: false,
        default_verbosity: None,
        apply_patch_tool_type: None,
        web_search_tool_type: WebSearchToolType::Text,
        truncation_policy: TruncationPolicyConfig::bytes(/*limit*/ 10_000),
        supports_parallel_tool_calls: false,
        supports_image_detail_original: false,
        context_window: model.max_input_tokens,
        max_context_window: model.max_input_tokens,
        auto_compact_token_limit: None,
        effective_context_window_percent: 95,
        additional_speed_tiers: Vec::new(),
        experimental_supported_tools: Vec::new(),
        input_modalities,
        used_fallback_model_metadata: false,
        supports_search_tool: false,
    }
}

fn anthropic_supported_reasoning_levels(
    effort: &AnthropicEffortCapabilities,
) -> Vec<ReasoningEffortPreset> {
    if !effort.supported {
        return Vec::new();
    }

    [
        (ReasoningEffort::Low, effort.low.supported),
        (ReasoningEffort::Medium, effort.medium.supported),
        (ReasoningEffort::High, effort.high.supported),
        (ReasoningEffort::XHigh, effort.max_effort.supported),
    ]
    .into_iter()
    .filter(|(_, supported)| *supported)
    .map(|(effort, _)| ReasoningEffortPreset {
        effort,
        description: effort.to_string(),
    })
    .collect()
}

fn anthropic_description(capabilities: &AnthropicModelCapabilities) -> Option<String> {
    let mut capability_tags = vec!["Tool calling"];
    if capabilities.effort.supported {
        capability_tags.push("Reasoning effort");
    } else if capabilities.thinking.supported {
        capability_tags.push("Reasoning");
    }
    if capabilities.image_input.supported {
        capability_tags.push("Image input");
    }
    if capabilities.pdf_input.supported {
        capability_tags.push("PDF input");
    }
    if capabilities.structured_outputs.supported {
        capability_tags.push("Structured outputs");
    }
    Some(capability_tags.join(" • "))
}

fn openai_compatible_model_info_from_id(model: OpenAiCompatibleModel) -> ModelInfo {
    let catalog_entry = compatible_model_catalog_entry(model.id.as_str());
    let supported_parameters = merged_supported_parameters(&model, catalog_entry);
    let supported_reasoning_levels =
        supported_reasoning_levels_from_metadata(&supported_parameters, catalog_entry);
    let default_reasoning_level =
        (!supported_reasoning_levels.is_empty()).then_some(ReasoningEffort::Medium);
    let visibility = compatible_picker_visibility(&supported_parameters, catalog_entry);
    let priority = compatible_model_priority(&supported_parameters, catalog_entry);
    let description = compatible_description(&model, &supported_parameters, catalog_entry);
    let context_window = compatible_context_window(&model, catalog_entry);
    let input_modalities = compatible_input_modalities(&model, catalog_entry);
    let supports_search_tool = supports_search_parameter(&supported_parameters)
        || catalog_entry.is_some_and(|entry| entry.supports_search_tool);
    let display_name = model.name.clone().unwrap_or_else(|| model.id.clone());
    ModelInfo {
        slug: model.id,
        display_name,
        description,
        default_reasoning_level,
        supported_reasoning_levels,
        shell_type: ConfigShellToolType::Default,
        visibility,
        supported_in_api: true,
        priority,
        availability_nux: None,
        upgrade: None,
        base_instructions: String::new(),
        model_messages: None,
        supports_reasoning_summaries: false,
        default_reasoning_summary: ReasoningSummary::Auto,
        support_verbosity: false,
        default_verbosity: None,
        apply_patch_tool_type: None,
        web_search_tool_type: WebSearchToolType::Text,
        truncation_policy: TruncationPolicyConfig::bytes(/*limit*/ 10_000),
        supports_parallel_tool_calls: catalog_entry
            .is_some_and(|entry| entry.supports_parallel_tool_calls),
        supports_image_detail_original: false,
        context_window,
        max_context_window: context_window,
        auto_compact_token_limit: None,
        effective_context_window_percent: 95,
        additional_speed_tiers: Vec::new(),
        experimental_supported_tools: Vec::new(),
        input_modalities,
        used_fallback_model_metadata: true,
        supports_search_tool,
    }
}

fn merged_supported_parameters(
    model: &OpenAiCompatibleModel,
    catalog_entry: Option<&CompatibleModelCatalogEntry>,
) -> Vec<String> {
    if !model.supported_parameters.is_empty() {
        return model.supported_parameters.clone();
    }

    catalog_entry
        .map(|entry| entry.supported_parameters.clone())
        .unwrap_or_default()
}

fn supported_reasoning_levels_from_parameters(
    supported_parameters: &[String],
) -> Vec<ReasoningEffortPreset> {
    if !supports_reasoning_effort_parameter(supported_parameters) {
        return Vec::new();
    }

    [
        ReasoningEffort::Low,
        ReasoningEffort::Medium,
        ReasoningEffort::High,
    ]
    .into_iter()
    .map(|effort| ReasoningEffortPreset {
        effort,
        description: effort.to_string(),
    })
    .collect()
}

fn supported_reasoning_levels_from_metadata(
    supported_parameters: &[String],
    catalog_entry: Option<&CompatibleModelCatalogEntry>,
) -> Vec<ReasoningEffortPreset> {
    let levels = supported_reasoning_levels_from_parameters(supported_parameters);
    if !levels.is_empty() {
        return levels;
    }

    catalog_entry
        .map(|entry| entry.supported_reasoning_levels.clone())
        .unwrap_or_default()
}

fn compatible_description(
    model: &OpenAiCompatibleModel,
    supported_parameters: &[String],
    catalog_entry: Option<&CompatibleModelCatalogEntry>,
) -> Option<String> {
    let mut capability_tags = Vec::new();
    if tools_support_is_known(supported_parameters) {
        capability_tags.push(if supports_tool_parameter(supported_parameters) {
            "Tool calling"
        } else {
            "No tool calling"
        });
    }
    if supports_reasoning_effort_parameter(supported_parameters) {
        capability_tags.push("Reasoning effort");
    } else if supports_reasoning_parameter(supported_parameters) {
        capability_tags.push("Reasoning");
    }
    if supports_search_parameter(supported_parameters) {
        capability_tags.push("Search");
    }

    let capability_summary = (!capability_tags.is_empty()).then(|| capability_tags.join(" • "));
    let provider_or_catalog_description = model
        .description
        .as_deref()
        .or_else(|| catalog_entry.and_then(|entry| entry.description.as_deref()));
    match (provider_or_catalog_description, capability_summary) {
        (Some(description), Some(summary)) if !description.trim().is_empty() => {
            Some(format!("{summary} • {description}"))
        }
        (Some(description), None) if !description.trim().is_empty() => {
            Some(description.to_string())
        }
        (None, Some(summary)) => Some(summary),
        _ => None,
    }
}

fn compatible_context_window(
    model: &OpenAiCompatibleModel,
    catalog_entry: Option<&CompatibleModelCatalogEntry>,
) -> Option<i64> {
    model
        .context_length
        .or(model.context_window)
        .or_else(|| {
            model
                .top_provider
                .as_ref()
                .and_then(|provider| provider.context_length)
        })
        .or_else(|| catalog_entry.and_then(|entry| entry.context_window))
}

fn compatible_input_modalities(
    model: &OpenAiCompatibleModel,
    catalog_entry: Option<&CompatibleModelCatalogEntry>,
) -> Vec<InputModality> {
    let Some(architecture) = &model.architecture else {
        return catalog_entry
            .map(|entry| entry.input_modalities.clone())
            .filter(|modalities| !modalities.is_empty())
            .unwrap_or_else(|| vec![InputModality::Text]);
    };

    let mut modalities = Vec::new();
    if architecture
        .input_modalities
        .iter()
        .any(|modality| modality.eq_ignore_ascii_case("text"))
    {
        modalities.push(InputModality::Text);
    }
    if architecture
        .input_modalities
        .iter()
        .any(|modality| modality.eq_ignore_ascii_case("image"))
    {
        modalities.push(InputModality::Image);
    }
    if modalities.is_empty() {
        catalog_entry
            .map(|entry| entry.input_modalities.clone())
            .filter(|catalog_modalities| !catalog_modalities.is_empty())
            .unwrap_or_else(|| vec![InputModality::Text])
    } else {
        modalities
    }
}

fn compatible_picker_visibility(
    supported_parameters: &[String],
    catalog_entry: Option<&CompatibleModelCatalogEntry>,
) -> ModelVisibility {
    if catalog_entry.is_some_and(|entry| entry.visibility == ModelVisibility::Hide) {
        return ModelVisibility::Hide;
    }
    if tools_support_is_known(supported_parameters)
        && !supports_tool_parameter(supported_parameters)
    {
        return ModelVisibility::Hide;
    }
    if tools_support_is_known(supported_parameters) {
        return ModelVisibility::List;
    }
    catalog_entry
        .map(|entry| entry.visibility)
        .unwrap_or(ModelVisibility::Hide)
}

fn compatible_model_priority(
    supported_parameters: &[String],
    catalog_entry: Option<&CompatibleModelCatalogEntry>,
) -> i32 {
    if compatible_picker_visibility(supported_parameters, catalog_entry) != ModelVisibility::List {
        return 180;
    }
    if supports_reasoning_effort_parameter(supported_parameters) {
        return 40;
    }
    if supports_tool_parameter(supported_parameters) {
        return 50;
    }
    90
}

fn supports_tool_parameter(supported_parameters: &[String]) -> bool {
    has_supported_parameter(supported_parameters, "tools")
}

fn tools_support_is_known(supported_parameters: &[String]) -> bool {
    !supported_parameters.is_empty()
}

fn supports_reasoning_effort_parameter(supported_parameters: &[String]) -> bool {
    has_supported_parameter(supported_parameters, "reasoning_effort")
}

fn supports_reasoning_parameter(supported_parameters: &[String]) -> bool {
    has_supported_parameter(supported_parameters, "reasoning")
        || has_supported_parameter(supported_parameters, "include_reasoning")
}

fn supports_search_parameter(supported_parameters: &[String]) -> bool {
    has_supported_parameter(supported_parameters, "web_search_options")
}

fn has_supported_parameter(supported_parameters: &[String], needle: &str) -> bool {
    supported_parameters
        .iter()
        .any(|parameter| parameter.eq_ignore_ascii_case(needle))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::AuthProvider;
    use crate::provider::RetryConfig;
    use async_trait::async_trait;
    use codex_client::Request;
    use codex_client::Response;
    use codex_client::StreamResponse;
    use codex_client::TransportError;
    use http::HeaderMap;
    use http::StatusCode;
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::time::Duration;

    #[derive(Clone)]
    struct CapturingTransport {
        last_request: Arc<Mutex<Option<Request>>>,
        body: Arc<Vec<u8>>,
        etag: Option<String>,
    }

    impl Default for CapturingTransport {
        fn default() -> Self {
            Self {
                last_request: Arc::new(Mutex::new(None)),
                body: Arc::new(serde_json::to_vec(&ModelsResponse { models: Vec::new() }).unwrap()),
                etag: None,
            }
        }
    }

    #[async_trait]
    impl HttpTransport for CapturingTransport {
        async fn execute(&self, req: Request) -> Result<Response, TransportError> {
            *self.last_request.lock().unwrap() = Some(req);
            let mut headers = HeaderMap::new();
            if let Some(etag) = &self.etag {
                headers.insert(ETAG, etag.parse().unwrap());
            }
            Ok(Response {
                status: StatusCode::OK,
                headers,
                body: (*self.body).clone().into(),
            })
        }

        async fn stream(&self, _req: Request) -> Result<StreamResponse, TransportError> {
            Err(TransportError::Build("stream should not run".to_string()))
        }
    }

    #[derive(Clone, Default)]
    struct DummyAuth;

    impl AuthProvider for DummyAuth {
        fn add_auth_headers(&self, _headers: &mut HeaderMap) {}
    }

    fn provider(base_url: &str) -> Provider {
        Provider {
            name: "test".to_string(),
            base_url: base_url.to_string(),
            query_params: None,
            headers: HeaderMap::new(),
            retry: RetryConfig {
                max_attempts: 1,
                base_delay: Duration::from_millis(1),
                retry_429: false,
                retry_5xx: true,
                retry_transport: true,
            },
            stream_idle_timeout: Duration::from_secs(1),
        }
    }

    #[tokio::test]
    async fn appends_client_version_query() {
        let response = ModelsResponse { models: Vec::new() };

        let transport = CapturingTransport {
            last_request: Arc::new(Mutex::new(None)),
            body: Arc::new(serde_json::to_vec(&response).unwrap()),
            etag: None,
        };

        let client = ModelsClient::new(
            transport.clone(),
            provider("https://example.com/api/codex"),
            Arc::new(DummyAuth),
        );

        let (models, _) = client
            .list_models("0.99.0", HeaderMap::new())
            .await
            .expect("request should succeed");

        assert_eq!(models.len(), 0);

        let url = transport
            .last_request
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .url
            .clone();
        assert_eq!(
            url,
            "https://example.com/api/codex/models?client_version=0.99.0"
        );
    }

    #[tokio::test]
    async fn uses_v1_models_for_anthropic_root_base_url() {
        let response = ModelsResponse { models: Vec::new() };

        let transport = CapturingTransport {
            last_request: Arc::new(Mutex::new(None)),
            body: Arc::new(serde_json::to_vec(&response).unwrap()),
            etag: None,
        };

        let client = ModelsClient::new(
            transport.clone(),
            provider("https://api.anthropic.com"),
            Arc::new(DummyAuth),
        );

        let (models, _) = client
            .list_models("0.99.0", HeaderMap::new())
            .await
            .expect("request should succeed");

        assert_eq!(models.len(), 0);

        let url = transport
            .last_request
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .url
            .clone();
        assert_eq!(
            url,
            "https://api.anthropic.com/v1/models?client_version=0.99.0"
        );
    }

    #[tokio::test]
    async fn uses_v1_models_for_local_anthropic_proxy() {
        let response = ModelsResponse { models: Vec::new() };
        let transport = CapturingTransport {
            last_request: Arc::new(Mutex::new(None)),
            body: Arc::new(serde_json::to_vec(&response).unwrap()),
            etag: None,
        };
        let client = ModelsClient::new(
            transport.clone(),
            Provider {
                name: "Anthropic".to_string(),
                base_url: "http://127.0.0.1:4010".to_string(),
                query_params: None,
                headers: HeaderMap::new(),
                retry: RetryConfig {
                    max_attempts: 1,
                    base_delay: Duration::from_millis(1),
                    retry_429: false,
                    retry_5xx: true,
                    retry_transport: true,
                },
                stream_idle_timeout: Duration::from_secs(1),
            },
            Arc::new(DummyAuth),
        );

        let (models, _) = client
            .list_models("0.99.0", HeaderMap::new())
            .await
            .expect("request should succeed");
        assert_eq!(models.len(), 0);
        let request = transport.last_request.lock().unwrap().clone().unwrap();
        assert_eq!(
            request.url,
            "http://127.0.0.1:4010/v1/models?client_version=0.99.0"
        );
    }

    #[tokio::test]
    async fn parses_models_response() {
        let response = ModelsResponse {
            models: vec![
                serde_json::from_value(json!({
                    "slug": "gpt-test",
                    "display_name": "gpt-test",
                    "description": "desc",
                    "default_reasoning_level": "medium",
                    "supported_reasoning_levels": [{"effort": "low", "description": "low"}, {"effort": "medium", "description": "medium"}, {"effort": "high", "description": "high"}],
                    "shell_type": "shell_command",
                    "visibility": "list",
                    "minimal_client_version": [0, 99, 0],
                    "supported_in_api": true,
                    "priority": 1,
                    "upgrade": null,
                    "base_instructions": "base instructions",
                    "supports_reasoning_summaries": false,
                    "support_verbosity": false,
                    "default_verbosity": null,
                    "apply_patch_tool_type": null,
                    "truncation_policy": {"mode": "bytes", "limit": 10_000},
                    "supports_parallel_tool_calls": false,
                    "supports_image_detail_original": false,
                    "context_window": 272_000,
                    "experimental_supported_tools": [],
                }))
                .unwrap(),
            ],
        };

        let transport = CapturingTransport {
            last_request: Arc::new(Mutex::new(None)),
            body: Arc::new(serde_json::to_vec(&response).unwrap()),
            etag: None,
        };

        let client = ModelsClient::new(
            transport,
            provider("https://example.com/api/codex"),
            Arc::new(DummyAuth),
        );

        let (models, _) = client
            .list_models("0.99.0", HeaderMap::new())
            .await
            .expect("request should succeed");

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].slug, "gpt-test");
        assert_eq!(models[0].supported_in_api, true);
        assert_eq!(models[0].priority, 1);
    }

    #[tokio::test]
    async fn list_models_includes_etag() {
        let response = ModelsResponse { models: Vec::new() };

        let transport = CapturingTransport {
            last_request: Arc::new(Mutex::new(None)),
            body: Arc::new(serde_json::to_vec(&response).unwrap()),
            etag: Some("\"abc\"".to_string()),
        };

        let client = ModelsClient::new(
            transport,
            provider("https://example.com/api/codex"),
            Arc::new(DummyAuth),
        );

        let (models, etag) = client
            .list_models("0.1.0", HeaderMap::new())
            .await
            .expect("request should succeed");

        assert_eq!(models.len(), 0);
        assert_eq!(etag, Some("\"abc\"".to_string()));
    }

    #[tokio::test]
    async fn parses_openai_compatible_models_response() {
        let transport = CapturingTransport {
            last_request: Arc::new(Mutex::new(None)),
            body: Arc::new(
                serde_json::to_vec(&serde_json::json!({
                    "object": "list",
                    "data": [
                        {
                            "id": "llama-3.3-70b-versatile",
                            "object": "model",
                            "context_window": 131072
                        }
                    ]
                }))
                .unwrap(),
            ),
            etag: None,
        };

        let client = ModelsClient::new(
            transport,
            provider("https://example.com/v1"),
            Arc::new(DummyAuth),
        );

        let (models, _) = client
            .list_models("0.1.0", HeaderMap::new())
            .await
            .expect("request should succeed");

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].slug, "llama-3.3-70b-versatile".to_string());
        assert_eq!(
            models[0].display_name,
            "llama-3.3-70b-versatile".to_string()
        );
        assert_eq!(models[0].context_window, Some(131072));
        assert!(models[0].used_fallback_model_metadata);
        assert_eq!(models[0].input_modalities, vec![InputModality::Text]);
        assert_eq!(models[0].visibility, ModelVisibility::List);
    }

    #[tokio::test]
    async fn defaults_openai_compatible_models_to_text_only_when_modalities_are_missing() {
        let transport = CapturingTransport {
            last_request: Arc::new(Mutex::new(None)),
            body: Arc::new(
                serde_json::to_vec(&serde_json::json!({
                    "data": [
                        {
                            "id": "openrouter/minimal-text-model",
                            "architecture": {}
                        }
                    ]
                }))
                .unwrap(),
            ),
            etag: None,
        };

        let client = ModelsClient::new(
            transport,
            provider("https://api.example.com/v1"),
            Arc::new(DummyAuth),
        );

        let (models, _) = client
            .list_models("0.1.0", HeaderMap::new())
            .await
            .expect("request should succeed");

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].input_modalities, vec![InputModality::Text]);
    }

    #[tokio::test]
    async fn parses_openrouter_compatible_models_response_with_capabilities() {
        let transport = CapturingTransport {
            last_request: Arc::new(Mutex::new(None)),
            body: Arc::new(
                serde_json::to_vec(&serde_json::json!({
                    "data": [
                        {
                            "id": "openai/gpt-5.4-mini",
                            "name": "OpenAI: GPT-5.4 Mini",
                            "description": "Fast model for coding",
                            "context_length": 400000,
                            "supported_parameters": [
                                "tools",
                                "tool_choice",
                                "reasoning_effort",
                                "web_search_options"
                            ],
                            "architecture": {
                                "input_modalities": ["text"]
                            }
                        }
                    ]
                }))
                .unwrap(),
            ),
            etag: None,
        };

        let client = ModelsClient::new(
            transport,
            provider("https://openrouter.ai/api/v1"),
            Arc::new(DummyAuth),
        );

        let (models, _) = client
            .list_models("0.1.0", HeaderMap::new())
            .await
            .expect("request should succeed");

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].slug, "openai/gpt-5.4-mini".to_string());
        assert_eq!(models[0].display_name, "OpenAI: GPT-5.4 Mini".to_string());
        assert!(
            models[0]
                .description
                .as_deref()
                .is_some_and(|description| description.contains("Tool calling"))
        );
        assert_eq!(models[0].context_window, Some(400000));
        assert_eq!(
            models[0].default_reasoning_level,
            Some(ReasoningEffort::Medium)
        );
        assert_eq!(
            models[0].supported_reasoning_levels,
            vec![
                ReasoningEffortPreset {
                    effort: ReasoningEffort::Low,
                    description: "low".to_string(),
                },
                ReasoningEffortPreset {
                    effort: ReasoningEffort::Medium,
                    description: "medium".to_string(),
                },
                ReasoningEffortPreset {
                    effort: ReasoningEffort::High,
                    description: "high".to_string(),
                },
            ]
        );
        assert_eq!(models[0].input_modalities, vec![InputModality::Text]);
        assert!(models[0].supports_search_tool);
        assert_eq!(models[0].visibility, ModelVisibility::List);
    }

    #[tokio::test]
    async fn hides_openai_compatible_models_when_capabilities_show_no_tool_calling() {
        let transport = CapturingTransport {
            last_request: Arc::new(Mutex::new(None)),
            body: Arc::new(
                serde_json::to_vec(&serde_json::json!({
                    "data": [
                        {
                            "id": "groq/compound-mini",
                            "supported_parameters": ["max_tokens", "temperature"]
                        }
                    ]
                }))
                .unwrap(),
            ),
            etag: None,
        };

        let client = ModelsClient::new(
            transport,
            provider("https://api.example.com/v1"),
            Arc::new(DummyAuth),
        );

        let (models, _) = client
            .list_models("0.1.0", HeaderMap::new())
            .await
            .expect("request should succeed");

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].visibility, ModelVisibility::Hide);
        assert!(
            models[0]
                .description
                .as_deref()
                .is_some_and(|description| description.contains("No tool calling"))
        );
    }

    #[tokio::test]
    async fn hides_unknown_compatible_models_when_no_capabilities_are_available() {
        let transport = CapturingTransport {
            last_request: Arc::new(Mutex::new(None)),
            body: Arc::new(
                serde_json::to_vec(&serde_json::json!({
                    "data": [
                        {
                            "id": "mystery-provider/mystery-model"
                        }
                    ]
                }))
                .unwrap(),
            ),
            etag: None,
        };

        let client = ModelsClient::new(
            transport,
            provider("https://api.example.com/v1"),
            Arc::new(DummyAuth),
        );

        let (models, _) = client
            .list_models("0.1.0", HeaderMap::new())
            .await
            .expect("request should succeed");

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].visibility, ModelVisibility::Hide);
    }

    #[tokio::test]
    async fn hides_catalog_models_that_are_not_codex_compatible() {
        let transport = CapturingTransport {
            last_request: Arc::new(Mutex::new(None)),
            body: Arc::new(
                serde_json::to_vec(&serde_json::json!({
                    "data": [
                        {
                            "id": "sora-2-pro"
                        }
                    ]
                }))
                .unwrap(),
            ),
            etag: None,
        };

        let client = ModelsClient::new(
            transport,
            provider("https://api.openai.com/v1"),
            Arc::new(DummyAuth),
        );

        let (models, _) = client
            .list_models("0.1.0", HeaderMap::new())
            .await
            .expect("request should succeed");

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].visibility, ModelVisibility::Hide);
    }

    #[tokio::test]
    async fn uses_catalog_capabilities_when_provider_omits_parameters() {
        let transport = CapturingTransport {
            last_request: Arc::new(Mutex::new(None)),
            body: Arc::new(
                serde_json::to_vec(&serde_json::json!({
                    "data": [
                        {
                            "id": "gpt-5",
                            "name": "GPT-5"
                        }
                    ]
                }))
                .unwrap(),
            ),
            etag: None,
        };

        let client = ModelsClient::new(
            transport,
            provider("https://api.openai.com/v1"),
            Arc::new(DummyAuth),
        );

        let (models, _) = client
            .list_models("0.1.0", HeaderMap::new())
            .await
            .expect("request should succeed");

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].visibility, ModelVisibility::List);
        assert!(
            models[0]
                .description
                .as_deref()
                .is_some_and(|description| description.contains("Tool calling"))
        );
        assert!(
            !models[0].supported_reasoning_levels.is_empty(),
            "catalog should surface reasoning support"
        );
    }

    #[tokio::test]
    async fn parses_anthropic_models_response_with_effort_and_toggle_capabilities() {
        let transport = CapturingTransport {
            last_request: Arc::new(Mutex::new(None)),
            body: Arc::new(
                serde_json::to_vec(&serde_json::json!({
                    "data": [
                        {
                            "id": "claude-sonnet-4-6",
                            "display_name": "Claude Sonnet 4.6",
                            "max_input_tokens": 1_000_000,
                            "capabilities": {
                                "effort": {
                                    "supported": true,
                                    "low": { "supported": true },
                                    "medium": { "supported": true },
                                    "high": { "supported": true },
                                    "max": { "supported": true }
                                },
                                "image_input": { "supported": true },
                                "pdf_input": { "supported": true },
                                "structured_outputs": { "supported": true },
                                "thinking": {
                                    "supported": true,
                                    "types": {
                                        "enabled": { "supported": true },
                                        "adaptive": { "supported": true }
                                    }
                                }
                            }
                        },
                        {
                            "id": "claude-haiku-4-5-20251001",
                            "display_name": "Claude Haiku 4.5",
                            "max_input_tokens": 200_000,
                            "capabilities": {
                                "effort": {
                                    "supported": false,
                                    "low": { "supported": false },
                                    "medium": { "supported": false },
                                    "high": { "supported": false },
                                    "max": { "supported": false }
                                },
                                "image_input": { "supported": true },
                                "pdf_input": { "supported": true },
                                "structured_outputs": { "supported": true },
                                "thinking": {
                                    "supported": true,
                                    "types": {
                                        "enabled": { "supported": true },
                                        "adaptive": { "supported": false }
                                    }
                                }
                            }
                        }
                    ]
                }))
                .unwrap(),
            ),
            etag: None,
        };

        let client = ModelsClient::new(
            transport,
            provider("https://api.anthropic.com"),
            Arc::new(DummyAuth),
        );

        let (models, _) = client
            .list_models("0.1.0", HeaderMap::new())
            .await
            .expect("request should succeed");

        let sonnet = models
            .iter()
            .find(|model| model.slug == "claude-sonnet-4-6")
            .expect("sonnet model");
        assert_eq!(
            sonnet.default_reasoning_level,
            Some(ReasoningEffort::Medium)
        );
        assert_eq!(
            sonnet
                .supported_reasoning_levels
                .iter()
                .map(|preset| preset.effort)
                .collect::<Vec<_>>(),
            vec![
                ReasoningEffort::Low,
                ReasoningEffort::Medium,
                ReasoningEffort::High,
                ReasoningEffort::XHigh,
            ]
        );

        let haiku = models
            .iter()
            .find(|model| model.slug == "claude-haiku-4-5-20251001")
            .expect("haiku model");
        assert_eq!(haiku.default_reasoning_level, Some(ReasoningEffort::Medium));
        assert!(haiku.supported_reasoning_levels.is_empty());
        assert_eq!(haiku.context_window, Some(200_000));
    }
}
