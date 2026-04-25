use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;
use codex_login::default_client::build_reqwest_client;
use codex_utils_string::take_bytes_at_char_boundary;
use regex_lite::Regex;
use reqwest::header::CONTENT_TYPE;
use serde::Deserialize;
use std::collections::HashSet;
use std::sync::OnceLock;
use url::Url;

use super::parse_arguments;

const BRAVE_SEARCH_URL: &str = "https://search.brave.com/search";
const KIMI_WEB_RESULT_LIMIT: usize = 20;
const KIMI_WEB_FETCH_MAX_BYTES: usize = 20_000;
const KIMI_WEB_USER_AGENT: &str = "Mozilla/5.0 (compatible; OpenInterpreter/0.0; +https://github.com/openinterpreter/open-interpreter)";

pub struct KimiFetchUrlHandler;
pub struct KimiSearchWebHandler;

#[derive(Deserialize)]
struct KimiSearchWebArgs {
    query: String,
    limit: Option<usize>,
    include_content: Option<bool>,
}

#[derive(Deserialize)]
struct KimiFetchUrlArgs {
    url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SearchResult {
    title: String,
    url: String,
    snippet: String,
    content: Option<String>,
}

impl ToolHandler for KimiSearchWebHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolPayload::Function { arguments } = invocation.payload else {
            return Err(FunctionCallError::RespondToModel(
                "SearchWeb received unsupported payload".to_string(),
            ));
        };
        let args: KimiSearchWebArgs = parse_arguments(&arguments)?;
        let output = run_kimi_web_search(args).await?;
        Ok(FunctionToolOutput::from_text(output, Some(true)))
    }
}

impl ToolHandler for KimiFetchUrlHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolPayload::Function { arguments } = invocation.payload else {
            return Err(FunctionCallError::RespondToModel(
                "FetchURL received unsupported payload".to_string(),
            ));
        };
        let args: KimiFetchUrlArgs = parse_arguments(&arguments)?;
        let output = run_kimi_fetch_url(args).await?;
        Ok(FunctionToolOutput::from_text(output, Some(true)))
    }
}

async fn run_kimi_web_search(args: KimiSearchWebArgs) -> Result<String, FunctionCallError> {
    let query = args.query.trim().to_string();
    if query.is_empty() {
        return Err(FunctionCallError::RespondToModel(
            "SearchWeb query must not be empty".to_string(),
        ));
    }
    let client = build_reqwest_client();
    let response = client
        .get(BRAVE_SEARCH_URL)
        .query(&[("q", query.as_str()), ("source", "web")])
        .header("User-Agent", KIMI_WEB_USER_AGENT)
        .send()
        .await
        .map_err(|err| FunctionCallError::RespondToModel(format!("SearchWeb failed: {err}")))?;
    let response = response
        .error_for_status()
        .map_err(|err| FunctionCallError::RespondToModel(format!("SearchWeb failed: {err}")))?;
    let html = response
        .text()
        .await
        .map_err(|err| FunctionCallError::RespondToModel(format!("SearchWeb failed: {err}")))?;
    let limit = args.limit.unwrap_or(5).clamp(1, KIMI_WEB_RESULT_LIMIT);
    let results = parse_brave_search_results(&html, limit, args.include_content.unwrap_or(false));
    if results.is_empty() {
        return Ok("No results found.".to_string());
    }

    let mut lines = Vec::new();
    for (index, result) in results.iter().enumerate() {
        if index > 0 {
            lines.push("---".to_string());
            lines.push(String::new());
        }
        lines.push(format!("Title: {}", result.title));
        lines.push("Date: ".to_string());
        lines.push(format!("URL: {}", result.url));
        lines.push(format!("Summary: {}", result.snippet));
        if let Some(content) = &result.content
            && !content.is_empty()
        {
            lines.push(String::new());
            lines.push(content.clone());
        }
        lines.push(String::new());
    }
    Ok(lines.join("\n").trim().to_string())
}

async fn run_kimi_fetch_url(args: KimiFetchUrlArgs) -> Result<String, FunctionCallError> {
    let url = normalize_url(&args.url)?;
    let client = build_reqwest_client();
    let response = client
        .get(url.clone())
        .header("User-Agent", KIMI_WEB_USER_AGENT)
        .send()
        .await
        .map_err(|err| FunctionCallError::RespondToModel(format!("FetchURL failed: {err}")))?;
    let response = response
        .error_for_status()
        .map_err(|err| FunctionCallError::RespondToModel(format!("FetchURL failed: {err}")))?;
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let body = response
        .text()
        .await
        .map_err(|err| FunctionCallError::RespondToModel(format!("FetchURL failed: {err}")))?;

    let extracted =
        if content_type.starts_with("text/plain") || content_type.starts_with("text/markdown") {
            normalize_text(&decode_html_entities(&body))
        } else if content_type.contains("html") || body.contains("<html") {
            extract_readable_text_from_html(&body)
        } else {
            normalize_text(&decode_html_entities(&body))
        };
    Ok(take_truncated_text(&extracted, KIMI_WEB_FETCH_MAX_BYTES))
}

fn parse_brave_search_results(
    html: &str,
    limit: usize,
    include_content: bool,
) -> Vec<SearchResult> {
    let mut seen_urls = HashSet::new();
    let mut results = Vec::new();

    for captures in brave_result_regex().captures_iter(html) {
        let Some(url_match) = captures.get(1) else {
            continue;
        };
        let url = decode_html_entities(url_match.as_str());
        if Url::parse(&url).is_err() || !seen_urls.insert(url.clone()) {
            continue;
        }

        let title = captures
            .get(2)
            .map(|value| strip_html(value.as_str()))
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| url.clone());
        let snippet = captures
            .get(3)
            .map(|value| strip_html(value.as_str()))
            .unwrap_or_default();
        let content = include_content.then(|| snippet.clone());

        results.push(SearchResult {
            title,
            url,
            snippet,
            content,
        });
        if results.len() >= limit {
            break;
        }
    }

    results
}

fn normalize_url(raw: &str) -> Result<Url, FunctionCallError> {
    let trimmed = raw.trim();
    Url::parse(trimmed)
        .or_else(|_| Url::parse(&format!("https://{trimmed}")))
        .map_err(|err| FunctionCallError::RespondToModel(format!("FetchURL URL is invalid: {err}")))
}

fn take_truncated_text(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        text.to_string()
    } else {
        take_bytes_at_char_boundary(text, max_bytes).to_string()
    }
}

fn extract_readable_text_from_html(html: &str) -> String {
    let mut sanitized = html
        .replace("<br>", "\n")
        .replace("<br/>", "\n")
        .replace("<br />", "\n")
        .replace("</p>", "\n\n")
        .replace("</div>", "\n")
        .replace("</li>", "\n");
    sanitized = script_regex().replace_all(&sanitized, " ").into_owned();
    sanitized = style_regex().replace_all(&sanitized, " ").into_owned();
    sanitized = comment_regex().replace_all(&sanitized, " ").into_owned();
    sanitized = title_regex().replace_all(&sanitized, " ").into_owned();
    sanitized = tag_regex().replace_all(&sanitized, " ").into_owned();
    normalize_text(&decode_html_entities(&sanitized))
}

fn strip_html(input: &str) -> String {
    normalize_text(&decode_html_entities(&tag_regex().replace_all(input, " ")))
}

fn normalize_text(input: &str) -> String {
    whitespace_regex()
        .replace_all(input.trim(), " ")
        .trim()
        .to_string()
}

fn decode_html_entities(input: &str) -> String {
    input
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
}

fn brave_result_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(
            r#"class=\"snippet\"[^>]*data-type=\"web\".*?<a[^>]*href=\"([^\"]+)\"[^>]*>(.*?)</a>.*?<div class=\"snippet-description\">(.*?)</div>"#,
        )
        .unwrap_or_else(|_| unreachable!("valid brave search regex"))
    })
}

fn script_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(?is)<script.*?>.*?</script>")
            .unwrap_or_else(|_| unreachable!("valid script regex"))
    })
}

fn style_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(?is)<style.*?>.*?</style>")
            .unwrap_or_else(|_| unreachable!("valid style regex"))
    })
}

fn comment_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(?is)<!--.*?-->").unwrap_or_else(|_| unreachable!("valid comment regex"))
    })
}

fn title_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(?is)<title.*?>.*?</title>")
            .unwrap_or_else(|_| unreachable!("valid title regex"))
    })
}

fn tag_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(?is)<[^>]+>").unwrap_or_else(|_| unreachable!("valid tag regex"))
    })
}

fn whitespace_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"\s+").unwrap_or_else(|_| unreachable!("valid whitespace regex"))
    })
}
