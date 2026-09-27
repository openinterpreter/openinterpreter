//! Version-specific extraction contracts; v2 never decodes raw memory.

use codex_protocol::MemoryVersion;
use codex_secrets::redact_secrets;
use codex_utils_output_truncation::TruncationPolicy;
use codex_utils_output_truncation::truncate_text;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;

pub(crate) struct StageOneOutput {
    pub(crate) raw_memory: Option<String>,
    pub(crate) rollout_summary: String,
    pub(crate) rollout_slug: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyOutput {
    raw_memory: String,
    rollout_summary: String,
    #[serde(default)]
    rollout_slug: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SummaryOutput {
    rollout_summary: String,
    rollout_slug: String,
}

impl StageOneOutput {
    pub(crate) fn parse(source: &str, version: MemoryVersion) -> serde_json::Result<Self> {
        let (raw_memory, rollout_summary, rollout_slug) = match version {
            MemoryVersion::V1 => {
                let output: LegacyOutput = serde_json::from_str(source)?;
                (
                    Some(output.raw_memory),
                    output.rollout_summary,
                    output.rollout_slug,
                )
            }
            MemoryVersion::V2 => {
                let output: SummaryOutput = serde_json::from_str(source)?;
                (None, output.rollout_summary, Some(output.rollout_slug))
            }
        };
        let mut rollout_summary = redact_secrets(rollout_summary);
        if version == MemoryVersion::V2 {
            rollout_summary = truncate_text(&rollout_summary, TruncationPolicy::Bytes(9_000));
        }
        Ok(Self {
            raw_memory: raw_memory.map(redact_secrets),
            rollout_summary,
            rollout_slug: rollout_slug.map(redact_secrets),
        })
    }
}

pub(crate) fn output_schema(version: MemoryVersion) -> Value {
    match version {
        MemoryVersion::V1 => json!({
            "type": "object",
            "properties": {
                "rollout_summary": { "type": "string" },
                "rollout_slug": { "type": ["string", "null"] },
                "raw_memory": { "type": "string" }
            },
            "required": ["rollout_summary", "rollout_slug", "raw_memory"],
            "additionalProperties": false
        }),
        MemoryVersion::V2 => json!({
            "type": "object",
            "properties": {
                "rollout_summary": { "type": "string" },
                "rollout_slug": { "type": "string" }
            },
            "required": ["rollout_summary", "rollout_slug"],
            "additionalProperties": false
        }),
    }
}
