use codex_tools::AdditionalProperties;
use codex_tools::JsonSchema;
use codex_tools::ResponsesApiTool;
use codex_tools::ToolName;
use codex_tools::ToolSpec;
use regex_lite::Regex;
use serde::Deserialize;

use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolOutput;
use crate::tools::context::ToolPayload;
use crate::tools::context::boxed_tool_output;
use crate::tools::handlers::HarnessAliasHandler;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::CoreToolRuntime;
use crate::tools::registry::ToolExecutor;

#[derive(Clone, Copy)]
pub enum KimiCodeExtraHandler {
    AgentSwarm,
    FetchUrl,
}

impl ToolExecutor<ToolInvocation> for KimiCodeExtraHandler {
    fn tool_name(&self) -> ToolName {
        ToolName::plain(match self {
            Self::AgentSwarm => "AgentSwarm",
            Self::FetchUrl => "FetchURL",
        })
    }

    fn spec(&self) -> ToolSpec {
        let name = self.tool_name().name;
        ToolSpec::Function(ResponsesApiTool {
            name: name.clone(),
            description: format!("Open Interpreter Kimi Code compatibility alias for {name}."),
            strict: false,
            defer_loading: None,
            parameters: JsonSchema::object(
                Default::default(),
                /*required*/ None,
                Some(AdditionalProperties::from(true)),
            ),
            output_schema: None,
        })
    }

    fn supports_parallel_tool_calls(&self) -> bool {
        false
    }

    fn handle(&self, invocation: ToolInvocation) -> codex_tools::ToolExecutorFuture<'_> {
        Box::pin(async move {
            match self {
                Self::AgentSwarm => handle_agent_swarm(invocation).await,
                Self::FetchUrl => handle_fetch_url(invocation).await,
            }
        })
    }
}

impl CoreToolRuntime for KimiCodeExtraHandler {
    fn matches_kind(&self, payload: &ToolPayload) -> bool {
        matches!(payload, ToolPayload::Function { .. })
    }
}

#[derive(Deserialize)]
struct AgentSwarmArgs {
    description: String,
    items: Vec<String>,
    prompt_template: String,
    #[serde(default)]
    subagent_type: Option<String>,
}

async fn handle_agent_swarm(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let args: AgentSwarmArgs = parse_invocation_arguments(&invocation)?;
    if args.items.is_empty() {
        return text_output(
            "<agent_swarm_result>\n<summary>completed: 0</summary>\n</agent_swarm_result>",
        );
    }
    let mut outputs = Vec::with_capacity(args.items.len());
    for (index, item) in args.items.iter().enumerate() {
        let prompt = args.prompt_template.replace("{{item}}", item);
        let payload = ToolPayload::Function {
            arguments: serde_json::json!({
                "description": format!("{}: {item}", args.description),
                "prompt": prompt,
                "run_in_background": false,
                "subagent_type": args.subagent_type.as_deref().unwrap_or("coder"),
            })
            .to_string(),
        };
        let output = HarnessAliasHandler::Agent
            .handle(ToolInvocation {
                call_id: format!("{}-{index}", invocation.call_id),
                tool_name: ToolName::plain("Agent"),
                payload: payload.clone(),
                ..invocation.clone()
            })
            .await?;
        let result = output.code_mode_result(&payload);
        let text = result
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| result.to_string());
        let (agent_id, summary) = parse_foreground_agent_result(&text);
        outputs.push(format!(
            "<subagent agent_id=\"{}\" item=\"{}\" outcome=\"completed\">{}</subagent>",
            escape_xml_attribute(agent_id),
            escape_xml_attribute(item),
            summary
        ));
    }
    text_output(format!(
        "<agent_swarm_result>\n<summary>completed: {}</summary>\n{}\n</agent_swarm_result>",
        outputs.len(),
        outputs.join("\n")
    ))
}

fn parse_foreground_agent_result(output: &str) -> (&str, &str) {
    let agent_id = output
        .lines()
        .find_map(|line| line.strip_prefix("agent_id: "))
        .unwrap_or_default();
    let summary = output
        .split_once("\n[summary]\n")
        .map(|(_, summary)| summary.trim())
        .unwrap_or_else(|| output.trim());
    (agent_id, summary)
}

#[derive(Deserialize)]
struct FetchUrlArgs {
    url: String,
}

async fn handle_fetch_url(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let args: FetchUrlArgs = parse_invocation_arguments(&invocation)?;
    let url = reqwest::Url::parse(&args.url)
        .map_err(|err| FunctionCallError::RespondToModel(format!("Invalid URL: {err}")))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(FunctionCallError::RespondToModel(
            "FetchURL supports only http and https URLs.".to_string(),
        ));
    }
    let response = reqwest::Client::new()
        .get(url.clone())
        .send()
        .await
        .map_err(|err| FunctionCallError::RespondToModel(format!("FetchURL failed: {err}")))?;
    let status = response.status();
    if !status.is_success() {
        return Err(FunctionCallError::RespondToModel(format!(
            "FetchURL failed with HTTP {status}."
        )));
    }
    let body = response
        .text()
        .await
        .map_err(|err| FunctionCallError::RespondToModel(format!("FetchURL failed: {err}")))?;
    let content = extract_page_text(&body);
    text_output(format!(
        "The returned content is the main text extracted from the page. If you use it in your answer, cite this page as a markdown link, e.g. [title](url).\n\n{content}"
    ))
}

fn extract_page_text(html: &str) -> String {
    let Ok(script_regex) = Regex::new("(?is)<(script|style)[^>]*>.*?</(script|style)>") else {
        return html.to_string();
    };
    let Ok(block_regex) = Regex::new("(?i)</?(p|div|h[1-6]|li|br|article|section)[^>]*>") else {
        return html.to_string();
    };
    let Ok(tag_regex) = Regex::new("(?s)<[^>]+>") else {
        return html.to_string();
    };
    let without_scripts = script_regex.replace_all(html, " ");
    let with_breaks = block_regex.replace_all(&without_scripts, "\n");
    let without_tags = tag_regex.replace_all(&with_breaks, " ");
    let decoded = without_tags
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'");
    decoded
        .lines()
        .map(str::split_whitespace)
        .map(|parts| parts.collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse_invocation_arguments<T>(invocation: &ToolInvocation) -> Result<T, FunctionCallError>
where
    T: for<'de> Deserialize<'de>,
{
    let ToolPayload::Function { arguments } = &invocation.payload else {
        return Err(FunctionCallError::RespondToModel(
            "Kimi Code alias received unsupported tool payload".to_string(),
        ));
    };
    parse_arguments(arguments)
}

fn escape_xml_attribute(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn text_output(text: impl Into<String>) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        text.into(),
        Some(true),
    )))
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::extract_page_text;
    use super::parse_foreground_agent_result;

    #[test]
    fn extracts_agent_id_and_summary_for_swarm_results() {
        assert_eq!(
            parse_foreground_agent_result(
                "agent_id: abc-123\nactual_subagent_type: coder\nstatus: completed\n\n[summary]\nSWARM_A_OK"
            ),
            ("abc-123", "SWARM_A_OK")
        );
    }

    #[test]
    fn extracts_readable_page_text() {
        assert_eq!(
            extract_page_text(
                "<html><head><style>x{}</style></head><body><h1>Example &amp; Test</h1><p>Hello <b>world</b>.</p></body></html>"
            ),
            "Example & Test\nHello world ."
        );
    }
}
