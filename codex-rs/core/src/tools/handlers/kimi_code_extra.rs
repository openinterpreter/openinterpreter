use std::future::Future;
use std::io;
use std::net::IpAddr;
use std::net::SocketAddr;
use std::time::Duration;

use codex_network_proxy::is_non_public_ip;
use codex_tools::AdditionalProperties;
use codex_tools::JsonSchema;
use codex_tools::ResponsesApiTool;
use codex_tools::ToolName;
use codex_tools::ToolSpec;
use regex_lite::Regex;
use serde::Deserialize;
use tokio::net::lookup_host;
use tokio::time::timeout;
use url::Host;

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

    fn handle<'a>(&'a self, invocation: ToolInvocation) -> codex_tools::ToolExecutorFuture<'a>
    where
        ToolInvocation: 'a,
    {
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
        outputs.push(format!(
            "<subagent item=\"{}\" outcome=\"completed\">{}</subagent>",
            escape_xml_attribute(item),
            text.trim()
        ));
    }
    text_output(format!(
        "<agent_swarm_result>\n<summary>completed: {}</summary>\n{}\n</agent_swarm_result>",
        outputs.len(),
        outputs.join("\n")
    ))
}

#[derive(Deserialize)]
struct FetchUrlArgs {
    url: String,
}

const MAX_FETCH_URL_REDIRECTS: usize = 10;
const MAX_FETCH_BODY_BYTES: usize = 1_048_576;
const FETCH_URL_DNS_TIMEOUT: Duration = Duration::from_secs(2);
const FETCH_URL_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const FETCH_URL_NON_PUBLIC: &str = "FetchURL does not fetch private or loopback addresses.";
const FETCH_URL_SCHEME: &str = "FetchURL supports only http and https URLs.";
const FETCH_URL_INCOMPLETE: &str = "FetchURL requires a fully-formed public http or https URL.";
const FETCH_URL_UNVERIFIED: &str = "FetchURL could not verify that the URL is public.";

#[derive(Debug, Clone, PartialEq, Eq)]
struct PublicHttpTarget {
    pinned_dns: Option<PinnedDns>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PinnedDns {
    host: String,
    addrs: Vec<SocketAddr>,
}

async fn handle_fetch_url(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let args: FetchUrlArgs = parse_invocation_arguments(&invocation)?;
    let url = reqwest::Url::parse(&args.url)
        .map_err(|err| FunctionCallError::RespondToModel(format!("Invalid URL: {err}")))?;
    let body = fetch_url_with_lookup(url, |host, port| async move {
        lookup_host((host.as_str(), port))
            .await
            .map(Iterator::collect)
    })
    .await?;
    let content = extract_page_text(&body);
    text_output(format!(
        "The returned content is the main text extracted from the page. If you use it in your answer, cite this page as a markdown link, e.g. [title](url).\n\n{content}"
    ))
}

async fn fetch_url_with_lookup<F, Fut>(
    mut url: reqwest::Url,
    mut lookup: F,
) -> Result<String, FunctionCallError>
where
    F: FnMut(String, u16) -> Fut,
    Fut: Future<Output = io::Result<Vec<SocketAddr>>>,
{
    let mut redirects = 0;
    let response = loop {
        let target =
            resolve_public_http_url_with_lookup(&url, |host, port| lookup(host, port)).await?;
        let client = fetch_client_builder(target.pinned_dns.as_ref())
            .build()
            .map_err(|err| FunctionCallError::RespondToModel(format!("FetchURL failed: {err}")))?;
        let response =
            client.get(url.clone()).send().await.map_err(|err| {
                FunctionCallError::RespondToModel(format!("FetchURL failed: {err}"))
            })?;
        let status = response.status();
        if !status.is_redirection() {
            break response;
        }
        if redirects >= MAX_FETCH_URL_REDIRECTS {
            return Err(FunctionCallError::RespondToModel(
                "FetchURL failed: too many redirects.".to_string(),
            ));
        }
        let Some(location) = response.headers().get(reqwest::header::LOCATION) else {
            return Err(FunctionCallError::RespondToModel(format!(
                "FetchURL failed with HTTP {status}."
            )));
        };
        let location = location.to_str().map_err(|_| {
            FunctionCallError::RespondToModel(format!("FetchURL failed with HTTP {status}."))
        })?;
        url = url
            .join(location)
            .map_err(|err| FunctionCallError::RespondToModel(format!("Invalid URL: {err}")))?;
        redirects += 1;
    };

    let status = response.status();
    if !status.is_success() {
        return Err(FunctionCallError::RespondToModel(format!(
            "FetchURL failed with HTTP {status}."
        )));
    }
    read_fetch_body(response).await
}

fn fetch_client_builder(pinned_dns: Option<&PinnedDns>) -> reqwest::ClientBuilder {
    let mut builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(FETCH_URL_REQUEST_TIMEOUT);
    if let Some(PinnedDns { host, addrs }) = pinned_dns {
        builder = builder.resolve_to_addrs(host, addrs);
        let trimmed = host.trim_end_matches('.');
        if trimmed != host {
            builder = builder.resolve_to_addrs(trimmed, addrs);
        }
    }
    builder
}

async fn read_fetch_body(mut response: reqwest::Response) -> Result<String, FunctionCallError> {
    if response
        .content_length()
        .is_some_and(|len| len > MAX_FETCH_BODY_BYTES as u64)
    {
        return Err(FunctionCallError::RespondToModel(
            "FetchURL refused a response that exceeded the size limit.".to_string(),
        ));
    }

    let mut body = Vec::new();
    loop {
        if body.len() >= MAX_FETCH_BODY_BYTES {
            break;
        }
        match response.chunk().await {
            Ok(Some(chunk)) => {
                let remaining = MAX_FETCH_BODY_BYTES - body.len();
                if chunk.len() > remaining {
                    body.extend_from_slice(&chunk[..remaining]);
                    break;
                }
                body.extend_from_slice(&chunk);
            }
            Ok(None) => break,
            Err(err) => {
                return Err(FunctionCallError::RespondToModel(format!(
                    "FetchURL failed: {err}"
                )));
            }
        }
    }
    Ok(String::from_utf8_lossy(&body).into_owned())
}

async fn resolve_public_http_url_with_lookup<F, Fut>(
    url: &reqwest::Url,
    lookup: F,
) -> Result<PublicHttpTarget, FunctionCallError>
where
    F: FnOnce(String, u16) -> Fut,
    Fut: Future<Output = io::Result<Vec<SocketAddr>>>,
{
    if !matches!(url.scheme(), "http" | "https") {
        return Err(FunctionCallError::RespondToModel(
            FETCH_URL_SCHEME.to_string(),
        ));
    }
    match url.host() {
        None => Err(FunctionCallError::RespondToModel(
            FETCH_URL_INCOMPLETE.to_string(),
        )),
        Some(Host::Ipv4(ip)) => {
            reject_non_public_ip(IpAddr::V4(ip))?;
            Ok(PublicHttpTarget { pinned_dns: None })
        }
        Some(Host::Ipv6(ip)) => {
            reject_non_public_ip(IpAddr::V6(ip))?;
            Ok(PublicHttpTarget { pinned_dns: None })
        }
        Some(Host::Domain(domain)) => {
            let normalized = domain.trim_end_matches('.').to_ascii_lowercase();
            if normalized == "localhost" || normalized.ends_with(".localhost") {
                return Err(non_public_url_error());
            }
            if let Ok(ip) = normalized.parse::<IpAddr>() {
                reject_non_public_ip(ip)?;
                return Ok(PublicHttpTarget { pinned_dns: None });
            }
            let port = url.port_or_known_default().unwrap_or(80);
            let addrs = match timeout(FETCH_URL_DNS_TIMEOUT, lookup(normalized, port)).await {
                Ok(Ok(addrs)) if !addrs.is_empty() => addrs,
                _ => {
                    return Err(FunctionCallError::RespondToModel(
                        FETCH_URL_UNVERIFIED.to_string(),
                    ));
                }
            };
            if addrs.iter().any(|addr| is_non_public_ip(addr.ip())) {
                return Err(non_public_url_error());
            }
            let host = url.host_str().unwrap_or(domain).to_string();
            Ok(PublicHttpTarget {
                pinned_dns: Some(PinnedDns { host, addrs }),
            })
        }
    }
}

fn reject_non_public_ip(ip: IpAddr) -> Result<(), FunctionCallError> {
    if is_non_public_ip(ip) {
        Err(non_public_url_error())
    } else {
        Ok(())
    }
}

fn non_public_url_error() -> FunctionCallError {
    FunctionCallError::RespondToModel(FETCH_URL_NON_PUBLIC.to_string())
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
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&");
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
#[path = "kimi_code_fetch_url_tests.rs"]
mod fetch_url_tests;

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::extract_page_text;

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
