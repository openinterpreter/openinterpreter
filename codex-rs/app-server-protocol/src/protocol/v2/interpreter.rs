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
#[ts(export_to = "v2/")]
pub struct InterpreterModelSetParams {
    pub model: String,
    /// Explicit user consent to fetch this model's GGUF before selecting it.
    #[serde(default)]
    #[ts(optional)]
    pub download: bool,
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
    fn model_download_requires_explicit_opt_in() {
        let set: InterpreterModelSetParams =
            serde_json::from_value(serde_json::json!({"model": "example"})).unwrap();
        assert!(!set.download);
        let set: InterpreterModelSetParams =
            serde_json::from_value(serde_json::json!({"model": "example", "download": true}))
                .unwrap();
        assert!(set.download);
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
}
