use crate::JsonSchema;
use crate::TS;
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct FeedbackUploadParams {
    pub classification: String,
    #[ts(optional = nullable)]
    pub reason: Option<String>,
    /// Dedicated submission secret; never include this in the feedback reason or tags.
    #[ts(optional = nullable)]
    pub report_passcode: Option<String>,
    #[ts(optional = nullable)]
    pub thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub include_logs: bool,
    #[ts(optional = nullable)]
    pub extra_log_files: Option<Vec<PathBuf>>,
    #[ts(optional = nullable)]
    pub tags: Option<BTreeMap<String, String>>,
}

impl std::fmt::Debug for FeedbackUploadParams {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FeedbackUploadParams")
            .field("classification", &self.classification)
            .field("reason", &self.reason)
            .field(
                "report_passcode",
                &self.report_passcode.as_ref().map(|_| "<redacted>"),
            )
            .field("thread_id", &self.thread_id)
            .field("include_logs", &self.include_logs)
            .field("extra_log_files", &self.extra_log_files)
            .field("tags", &self.tags)
            .finish()
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct FeedbackUploadResponse {
    pub thread_id: String,
    /// Whitespace-normalized SHA-256 of the session base instructions, matching the
    /// uploaded `prompt_hash` tag. Does not include later developer messages.
    /// Null when the reported rollout has no prompt metadata.
    #[serde(default)]
    pub prompt_hash: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::FeedbackUploadParams;

    #[test]
    fn report_passcode_is_dedicated_rpc_field_and_debug_redacted() {
        let params: FeedbackUploadParams = serde_json::from_value(serde_json::json!({
            "classification": "bug",
            "reason": "ordinary note",
            "reportPasscode": "operator-secret-123",
            "includeLogs": false,
        }))
        .unwrap();
        let serialized = serde_json::to_value(&params).unwrap();
        assert_eq!(serialized["reason"], "ordinary note");
        assert_eq!(serialized["reportPasscode"], "operator-secret-123");
        assert!(!format!("{params:?}").contains("operator-secret-123"));
        assert_eq!(
            params.report_passcode.as_deref(),
            Some("operator-secret-123")
        );
    }
}
