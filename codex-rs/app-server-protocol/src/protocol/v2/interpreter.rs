use super::Model;
use crate::JsonSchema;
use crate::TS;
use codex_protocol::openai_models::ReasoningEffort;
use serde::Deserialize;
use serde::Serialize;

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export_to = "v2/")]
pub enum WireApiDto {
    Responses,
    Chat,
    Messages,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct InterpreterProvider {
    pub id: String,
    pub name: String,
    pub description: String,
    pub is_current: bool,
    #[ts(optional)]
    pub base_url: Option<String>,
    #[ts(optional)]
    pub wire_api: Option<WireApiDto>,
    #[ts(optional)]
    pub env_key: Option<String>,
    pub configured: bool,
    pub is_default: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct InterpreterHarness {
    #[ts(optional)]
    pub id: Option<String>,
    pub label: String,
    pub description: String,
    pub is_recommended: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct InterpreterProviderListParams {
    #[ts(optional = nullable)]
    pub include_unconfigured: Option<bool>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct InterpreterProviderListResponse {
    pub data: Vec<InterpreterProvider>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct InterpreterModelListParams {
    #[ts(optional = nullable)]
    pub model_provider: Option<String>,
    #[ts(optional = nullable)]
    pub include_hidden: Option<bool>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct InterpreterModelListResponse {
    pub data: Vec<Model>,
    /// Local GGUF availability is inspected through the same model catalog response.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(optional)]
    pub local_downloads: Vec<InterpreterLocalModelDownload>,
}

/// Explicit, user-initiated acquisition in the existing app-server protocol.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct InterpreterModelAcquireParams {
    pub model: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct InterpreterModelAcquireResponse {
    pub model: String,
    pub size_bytes: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct InterpreterLocalModelDownload {
    pub model: String,
    pub size_label: String,
    pub downloaded: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct InterpreterHarnessListParams {
    pub provider_id: String,
    #[ts(optional = nullable)]
    pub model: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct InterpreterHarnessListResponse {
    pub data: Vec<InterpreterHarness>,
}

/// Inspect effective harness inputs without changing the active configuration.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct InterpreterHarnessArgumentsInspectParams {
    pub provider_id: String,
    pub model: String,
    #[ts(optional = nullable)]
    pub harness: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct InterpreterHarnessArgumentsInspectResponse {
    pub provider_id: String,
    pub model: String,
    pub harness: String,
    /// Effective, read-only values from core; not a parallel settings store.
    pub arguments: serde_json::Value,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct InterpreterProviderSetParams {
    pub provider_id: String,
    #[ts(optional = nullable)]
    pub profile: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct InterpreterProviderSetResponse {}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
#[ts(export_to = "v2/")]
pub struct InterpreterModelSetParams {
    pub model: String,
    #[ts(optional = nullable)]
    pub reasoning_effort: Option<ReasoningEffort>,
    #[ts(optional = nullable)]
    pub profile: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct InterpreterModelSetResponse {}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct InterpreterHarnessSetParams {
    #[ts(optional = nullable)]
    pub harness: Option<String>,
    #[ts(optional = nullable)]
    pub profile: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct InterpreterHarnessSetResponse {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_download_is_separate_from_configuration() {
        let set: InterpreterModelSetParams =
            serde_json::from_value(serde_json::json!({"model": "example"})).unwrap();
        assert_eq!(set.model, "example");
        assert!(
            serde_json::from_value::<InterpreterModelSetParams>(
                serde_json::json!({"model": "example", "download": true})
            )
            .is_err()
        );
        let acquire: InterpreterModelAcquireParams =
            serde_json::from_value(serde_json::json!({"model": "example"})).unwrap();
        assert_eq!(acquire.model, "example");
    }

    #[test]
    fn local_availability_uses_existing_catalog_response() {
        let response: InterpreterModelListResponse =
            serde_json::from_value(serde_json::json!({"data": []})).unwrap();
        assert!(response.local_downloads.is_empty());
        let availability = InterpreterLocalModelDownload {
            model: "example".to_string(),
            size_label: "1 GB".to_string(),
            downloaded: false,
        };
        assert_eq!(
            serde_json::to_value(availability).unwrap(),
            serde_json::json!({"model": "example", "sizeLabel": "1 GB", "downloaded": false})
        );
    }

    #[test]
    fn harness_arguments_inspect_is_read_only_and_typed() {
        let params: InterpreterHarnessArgumentsInspectParams = serde_json::from_value(
            serde_json::json!({"providerId": "local", "model": "example", "harness": "local"}),
        )
        .unwrap();
        let response = InterpreterHarnessArgumentsInspectResponse {
            provider_id: params.provider_id,
            model: params.model,
            harness: params.harness.unwrap(),
            arguments: serde_json::json!({"contextWindow": 32768}),
        };
        assert_eq!(
            serde_json::to_value(response).unwrap()["arguments"]["contextWindow"],
            32768
        );
    }
}
