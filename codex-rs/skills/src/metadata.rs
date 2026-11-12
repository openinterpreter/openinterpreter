//! Parses optional skill YAML metadata with shared host and executor validation.

use codex_protocol::protocol::Product;
use serde::Deserialize;

use crate::SkillDependencies;
use crate::SkillInterfaceFile;
use crate::SkillPolicy;
use crate::SkillToolDependency;

const MAX_DEPENDENCY_TYPE_LEN: usize = 64;
const MAX_DEPENDENCY_TRANSPORT_LEN: usize = 64;
const MAX_DEPENDENCY_VALUE_LEN: usize = 1024;
const MAX_DEPENDENCY_DESCRIPTION_LEN: usize = 1024;
const MAX_DEPENDENCY_COMMAND_LEN: usize = 1024;
const MAX_DEPENDENCY_URL_LEN: usize = 1024;

/// Parsed optional metadata; callers resolve interface assets for their filesystem.
#[derive(Debug, Default)]
pub struct ParsedSkillMetadata {
    pub interface: Option<SkillInterfaceFile>,
    pub dependencies: Option<SkillDependencies>,
    pub policy: Option<SkillPolicy>,
}

/// Parses `agents/openai.yaml`, dropping invalid individual dependency declarations.
pub fn parse_skill_metadata(contents: &str) -> Result<ParsedSkillMetadata, serde_yaml::Error> {
    let parsed: SkillMetadataFile = serde_yaml::from_str(contents)?;
    Ok(ParsedSkillMetadata {
        interface: parsed.interface,
        dependencies: resolve_dependencies(parsed.dependencies),
        policy: resolve_policy(parsed.policy),
    })
}

#[derive(Debug, Default, Deserialize)]
struct SkillMetadataFile {
    #[serde(default)]
    interface: Option<SkillInterfaceFile>,
    #[serde(default)]
    dependencies: Option<Dependencies>,
    #[serde(default)]
    policy: Option<Policy>,
}

#[derive(Debug, Default, Deserialize)]
struct Dependencies {
    #[serde(default)]
    tools: Vec<DependencyTool>,
}

#[derive(Debug, Deserialize)]
struct Policy {
    #[serde(default)]
    allow_implicit_invocation: Option<bool>,
    #[serde(default)]
    products: Vec<Product>,
}

#[derive(Debug, Default, Deserialize)]
struct DependencyTool {
    #[serde(rename = "type")]
    kind: Option<String>,
    value: Option<String>,
    description: Option<String>,
    transport: Option<String>,
    command: Option<String>,
    url: Option<String>,
    oauth: Option<DependencyOAuth>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DependencyOAuth {
    #[serde(alias = "callback_port")]
    callback_port: Option<u16>,
}

fn resolve_dependencies(dependencies: Option<Dependencies>) -> Option<SkillDependencies> {
    let dependencies = dependencies?;
    let tools = dependencies
        .tools
        .into_iter()
        .filter_map(resolve_dependency_tool)
        .collect::<Vec<_>>();
    if tools.is_empty() {
        None
    } else {
        Some(SkillDependencies { tools })
    }
}

fn resolve_policy(policy: Option<Policy>) -> Option<SkillPolicy> {
    policy.map(|policy| SkillPolicy {
        allow_implicit_invocation: policy.allow_implicit_invocation,
        products: policy.products,
    })
}

fn resolve_dependency_tool(tool: DependencyTool) -> Option<SkillToolDependency> {
    let r#type = resolve_required_str(
        tool.kind,
        MAX_DEPENDENCY_TYPE_LEN,
        "dependencies.tools.type",
    )?;
    let value = resolve_required_str(
        tool.value,
        MAX_DEPENDENCY_VALUE_LEN,
        "dependencies.tools.value",
    )?;
    let description = resolve_str(
        tool.description,
        MAX_DEPENDENCY_DESCRIPTION_LEN,
        "dependencies.tools.description",
    );
    let transport = resolve_str(
        tool.transport,
        MAX_DEPENDENCY_TRANSPORT_LEN,
        "dependencies.tools.transport",
    );
    let command = resolve_str(
        tool.command,
        MAX_DEPENDENCY_COMMAND_LEN,
        "dependencies.tools.command",
    );
    let url = resolve_str(tool.url, MAX_DEPENDENCY_URL_LEN, "dependencies.tools.url");
    let oauth_callback_port = tool.oauth.and_then(|oauth| oauth.callback_port);

    Some(SkillToolDependency {
        r#type,
        value,
        description,
        transport,
        command,
        url,
        oauth_callback_port,
    })
}

fn resolve_str(value: Option<String>, max_len: usize, field: &'static str) -> Option<String> {
    let value = value?;
    let value = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if value.is_empty() {
        tracing::warn!("ignoring {field}: value is empty");
        return None;
    }
    if value.chars().count() > max_len {
        tracing::warn!("ignoring {field}: exceeds maximum length of {max_len} characters");
        return None;
    }
    Some(value)
}

fn resolve_required_str(
    value: Option<String>,
    max_len: usize,
    field: &'static str,
) -> Option<String> {
    let Some(value) = value else {
        tracing::warn!("ignoring {field}: value is missing");
        return None;
    };
    resolve_str(Some(value), max_len, field)
}
