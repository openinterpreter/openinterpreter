use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AnthropicMessageRequest {
    pub model: String,
    pub messages: Vec<AnthropicMessage>,
    pub system: Vec<AnthropicTextBlock>,
    pub tools: Vec<AnthropicTool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<AnthropicThinkingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_management: Option<AnthropicContextManagement>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_config: Option<AnthropicOutputConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<AnthropicRequestMetadata>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<u32>,
    pub max_tokens: u32,
    pub stream: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AnthropicMessage {
    pub role: String,
    pub content: AnthropicMessageContent,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(untagged)]
pub enum AnthropicMessageContent {
    Text(String),
    Blocks(Vec<AnthropicContentBlock>),
}

impl AnthropicMessageContent {
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Text(text) => text.is_empty(),
            Self::Blocks(blocks) => blocks.is_empty(),
        }
    }

    pub fn blocks(&self) -> Option<&[AnthropicContentBlock]> {
        match self {
            Self::Text(_) => None,
            Self::Blocks(blocks) => Some(blocks.as_slice()),
        }
    }

    pub fn blocks_mut(&mut self) -> Option<&mut Vec<AnthropicContentBlock>> {
        match self {
            Self::Text(_) => None,
            Self::Blocks(blocks) => Some(blocks),
        }
    }
}

impl From<Vec<AnthropicContentBlock>> for AnthropicMessageContent {
    fn from(value: Vec<AnthropicContentBlock>) -> Self {
        Self::Blocks(value)
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AnthropicTextBlock {
    #[serde(rename = "type")]
    pub block_type: &'static str,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<AnthropicCacheControl>,
}

impl AnthropicTextBlock {
    pub fn new(text: String) -> Self {
        Self {
            block_type: "text",
            text,
            cache_control: None,
        }
    }

    pub fn ephemeral(text: String) -> Self {
        Self {
            block_type: "text",
            text,
            cache_control: Some(AnthropicCacheControl::ephemeral()),
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AnthropicContentBlock {
    Text {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<AnthropicCacheControl>,
    },
    Thinking {
        thinking: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
    },
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        tool_use_id: String,
        content: AnthropicToolResultContent,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<AnthropicCacheControl>,
    },
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(untagged)]
pub enum AnthropicToolResultContent {
    Text(String),
    Blocks(Vec<AnthropicToolResultBlock>),
}

/// A single block inside a tool result. Anthropic tool results may carry text
/// or image blocks; image blocks are what `Read` returns for image files.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AnthropicToolResultBlock {
    Text {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<AnthropicCacheControl>,
    },
    Image {
        source: AnthropicImageSource,
    },
}

/// A base64-encoded image source, matching the Anthropic Messages API shape:
/// `{"type":"base64","media_type":"image/png","data":"..."}`.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AnthropicImageSource {
    #[serde(rename = "type")]
    pub source_type: &'static str,
    pub media_type: String,
    pub data: String,
}

impl AnthropicImageSource {
    pub fn base64(media_type: String, data: String) -> Self {
        Self {
            source_type: "base64",
            media_type,
            data,
        }
    }
}

impl From<String> for AnthropicToolResultContent {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<Vec<AnthropicToolResultBlock>> for AnthropicToolResultContent {
    fn from(value: Vec<AnthropicToolResultBlock>) -> Self {
        Self::Blocks(value)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct AnthropicTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AnthropicThinkingConfig {
    #[serde(rename = "type")]
    pub config_type: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget_tokens: Option<u32>,
}

impl AnthropicThinkingConfig {
    pub fn adaptive() -> Self {
        Self {
            config_type: "adaptive",
            budget_tokens: None,
        }
    }

    pub fn enabled(budget_tokens: u32) -> Self {
        Self {
            config_type: "enabled",
            budget_tokens: Some(budget_tokens),
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AnthropicContextManagement {
    pub edits: Vec<AnthropicContextEdit>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AnthropicContextEdit {
    #[serde(rename = "type")]
    pub edit_type: &'static str,
    pub keep: &'static str,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AnthropicOutputConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<AnthropicOutputFormat>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AnthropicRequestMetadata {
    pub user_id: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AnthropicOutputFormat {
    JsonSchema { schema: Value },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AnthropicCacheControl {
    #[serde(rename = "type")]
    pub cache_type: &'static str,
}

impl AnthropicCacheControl {
    pub fn ephemeral() -> Self {
        Self {
            cache_type: "ephemeral",
        }
    }
}
