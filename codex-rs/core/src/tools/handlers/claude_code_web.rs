use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;
use codex_login::default_client::build_reqwest_client;
use codex_protocol::models::WebSearchAction;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::WebSearchBeginEvent;
use codex_protocol::protocol::WebSearchEndEvent;
use codex_utils_string::take_bytes_at_char_boundary;
use regex_lite::Regex;
use reqwest::header::CONTENT_TYPE;
use serde::Deserialize;
use std::collections::HashSet;
use std::sync::OnceLock;
use url::Url;

use super::parse_arguments;

const BRAVE_SEARCH_URL: &str = "https://search.brave.com/search";
const CLAUDE_WEB_RESULT_LIMIT: usize = 8;
const CLAUDE_WEB_FETCH_MAX_BYTES: usize = 20_000;
const CLAUDE_WEB_USER_AGENT: &str = "Mozilla/5.0 (compatible; OpenInterpreter/0.0; +https://github.com/openinterpreter/open-interpreter)";

pub struct ClaudeWebFetchHandler;
pub struct ClaudeWebSearchHandler;

#[derive(Deserialize)]
struct ClaudeWebFetchArgs {
    url: String,
    prompt: String,
}

#[derive(Deserialize)]
struct ClaudeWebSearchArgs {
    query: String,
    allowed_domains: Option<Vec<String>>,
    blocked_domains: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SearchResult {
    title: String,
    url: String,
    snippet: String,
}

impl ToolHandler for ClaudeWebSearchHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolInvocation {
            session,
            turn,
            call_id,
            payload,
            ..
        } = invocation;
        let ToolPayload::Function { arguments } = payload else {
            return Err(FunctionCallError::RespondToModel(
                "WebSearch received unsupported payload".to_string(),
            ));
        };
        let args: ClaudeWebSearchArgs = parse_arguments(&arguments)?;
        let query = args.query.trim().to_string();
        if query.len() < 2 {
            return Err(FunctionCallError::RespondToModel(
                "WebSearch query must be at least 2 characters".to_string(),
            ));
        }

        session
            .send_event(
                turn.as_ref(),
                EventMsg::WebSearchBegin(WebSearchBeginEvent {
                    call_id: call_id.clone(),
                }),
            )
            .await;

        let result = run_web_search(&query, args.allowed_domains, args.blocked_domains).await;

        session
            .send_event(
                turn.as_ref(),
                EventMsg::WebSearchEnd(WebSearchEndEvent {
                    call_id,
                    query: query.clone(),
                    action: WebSearchAction::Search {
                        query: Some(query.clone()),
                        queries: None,
                    },
                }),
            )
            .await;

        result.map(|text| FunctionToolOutput::from_text(text, Some(true)))
    }
}

impl ToolHandler for ClaudeWebFetchHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolPayload::Function { arguments } = invocation.payload else {
            return Err(FunctionCallError::RespondToModel(
                "WebFetch received unsupported payload".to_string(),
            ));
        };
        let args: ClaudeWebFetchArgs = parse_arguments(&arguments)?;
        let output = run_web_fetch(args).await?;
        Ok(FunctionToolOutput::from_text(output, Some(true)))
    }
}

async fn run_web_search(
    query: &str,
    allowed_domains: Option<Vec<String>>,
    blocked_domains: Option<Vec<String>>,
) -> Result<String, FunctionCallError> {
    let client = build_reqwest_client();
    let response = client
        .get(BRAVE_SEARCH_URL)
        .query(&[("q", query), ("source", "web")])
        .header("User-Agent", CLAUDE_WEB_USER_AGENT)
        .send()
        .await
        .map_err(|err| FunctionCallError::RespondToModel(format!("WebSearch failed: {err}")))?;
    let response = response
        .error_for_status()
        .map_err(|err| FunctionCallError::RespondToModel(format!("WebSearch failed: {err}")))?;
    let html = response
        .text()
        .await
        .map_err(|err| FunctionCallError::RespondToModel(format!("WebSearch failed: {err}")))?;

    let results = parse_brave_search_results(
        &html,
        allowed_domains.as_deref(),
        blocked_domains.as_deref(),
    );
    Ok(format_web_search_results(query, &results))
}

async fn run_web_fetch(args: ClaudeWebFetchArgs) -> Result<String, FunctionCallError> {
    let requested_url = normalize_web_fetch_url(&args.url)?;
    let requested_host = requested_url.host_str().map(str::to_ascii_lowercase);
    let client = build_reqwest_client();
    let response = client
        .get(requested_url.clone())
        .header("User-Agent", CLAUDE_WEB_USER_AGENT)
        .send()
        .await
        .map_err(|err| FunctionCallError::RespondToModel(format!("WebFetch failed: {err}")))?;
    let response = response
        .error_for_status()
        .map_err(|err| FunctionCallError::RespondToModel(format!("WebFetch failed: {err}")))?;

    let final_url = response.url().clone();
    let final_host = final_url.host_str().map(str::to_ascii_lowercase);
    if requested_host.as_deref() != final_host.as_deref() {
        return Ok(format!(
            "The URL redirected to a different host. Make a new WebFetch request with this redirect URL:\n{}",
            final_url.as_str()
        ));
    }

    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let body = response
        .text()
        .await
        .map_err(|err| FunctionCallError::RespondToModel(format!("WebFetch failed: {err}")))?;

    let extracted = if content_type.contains("html") || body.contains("<html") {
        extract_readable_text_from_html(&body)
    } else {
        normalize_text(&decode_html_entities(&body))
    };
    let truncated = extracted.len() > CLAUDE_WEB_FETCH_MAX_BYTES;
    let content = take_bytes_at_char_boundary(&extracted, CLAUDE_WEB_FETCH_MAX_BYTES).to_string();

    Ok(format_web_fetch_output(
        final_url.as_str(),
        &args.prompt,
        &content,
        truncated,
    ))
}

fn normalize_web_fetch_url(raw: &str) -> Result<Url, FunctionCallError> {
    let trimmed = raw.trim();
    let mut url = Url::parse(trimmed)
        .or_else(|_| Url::parse(&format!("https://{trimmed}")))
        .map_err(|err| {
            FunctionCallError::RespondToModel(format!("WebFetch URL is invalid: {err}"))
        })?;

    if url.scheme() == "http" {
        url.set_scheme("https").map_err(|_| {
            FunctionCallError::RespondToModel("WebFetch could not upgrade URL to HTTPS".to_string())
        })?;
    }

    if url.scheme() != "https" {
        return Err(FunctionCallError::RespondToModel(
            "WebFetch only supports HTTP(S) URLs".to_string(),
        ));
    }

    if url.host_str().is_none() {
        return Err(FunctionCallError::RespondToModel(
            "WebFetch URL must include a host".to_string(),
        ));
    }

    Ok(url)
}

fn parse_brave_search_results(
    html: &str,
    allowed_domains: Option<&[String]>,
    blocked_domains: Option<&[String]>,
) -> Vec<SearchResult> {
    let allowed_domains = allowed_domains.map(normalize_domains);
    let blocked_domains = blocked_domains.map(normalize_domains);
    let mut seen_urls = HashSet::new();
    let mut results = Vec::new();

    for captures in brave_result_regex().captures_iter(html) {
        let Some(url_match) = captures.get(1) else {
            continue;
        };
        let url = decode_html_entities(url_match.as_str());
        let Ok(parsed_url) = Url::parse(&url) else {
            continue;
        };
        let Some(host) = parsed_url.host_str().map(str::to_ascii_lowercase) else {
            continue;
        };
        if !domain_allowed(
            &host,
            allowed_domains.as_deref(),
            blocked_domains.as_deref(),
        ) {
            continue;
        }
        if !seen_urls.insert(url.clone()) {
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

        results.push(SearchResult {
            title,
            url,
            snippet,
        });
        if results.len() >= CLAUDE_WEB_RESULT_LIMIT {
            break;
        }
    }

    results
}

fn format_web_search_results(query: &str, results: &[SearchResult]) -> String {
    if results.is_empty() {
        return format!("No public web search results matched `{query}`.");
    }

    let mut lines = vec![format!("Search results for `{query}`:"), String::new()];
    for (index, result) in results.iter().enumerate() {
        lines.push(format!("{}. [{}]({})", index + 1, result.title, result.url));
        if !result.snippet.is_empty() {
            lines.push(format!("   {}", result.snippet));
        }
        lines.push(String::new());
    }
    lines.join("\n").trim().to_string()
}

fn format_web_fetch_output(url: &str, prompt: &str, content: &str, truncated: bool) -> String {
    let truncation_notice = if truncated {
        "\n\n[Content truncated before returning to the model.]"
    } else {
        ""
    };
    format!("Fetched URL: {url}\nPrompt: {prompt}\n\nContent:\n{content}{truncation_notice}")
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
    let title = title_regex()
        .captures(&sanitized)
        .and_then(|captures| captures.get(1).map(|value| strip_html(value.as_str())));
    sanitized = title_regex().replace_all(&sanitized, " ").into_owned();
    sanitized = tag_regex().replace_all(&sanitized, " ").into_owned();
    sanitized = decode_html_entities(&sanitized);
    let body = normalize_text(&sanitized);

    match title.filter(|value| !value.is_empty()) {
        Some(title) if !body.is_empty() => format!("Title: {title}\n\n{body}"),
        Some(title) => title,
        None => body,
    }
}

fn normalize_domains(domains: &[String]) -> Vec<String> {
    domains
        .iter()
        .map(|domain| domain.trim().trim_start_matches('.').to_ascii_lowercase())
        .filter(|domain| !domain.is_empty())
        .collect()
}

fn domain_allowed(
    host: &str,
    allowed_domains: Option<&[String]>,
    blocked_domains: Option<&[String]>,
) -> bool {
    if blocked_domains
        .is_some_and(|domains| domains.iter().any(|domain| domain_matches(host, domain)))
    {
        return false;
    }
    allowed_domains.is_none_or(|domains| domains.iter().any(|domain| domain_matches(host, domain)))
}

fn domain_matches(host: &str, domain: &str) -> bool {
    host == domain || host.ends_with(&format!(".{domain}"))
}

fn strip_html(input: &str) -> String {
    normalize_text(&decode_html_entities(&tag_regex().replace_all(input, " ")))
}

fn normalize_text(input: &str) -> String {
    let mut text = whitespace_regex().replace_all(input, " ").into_owned();
    text = blank_line_regex().replace_all(&text, "\n\n").into_owned();
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn decode_html_entities(input: &str) -> String {
    input
        .replace("&nbsp;", " ")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&quot;", "\"")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn brave_result_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r#"(?s)<div class="snippet[^"]*"[^>]*data-type="web"[^>]*>.*?<a href="(https?://[^"]+)"[^>]*>.*?<div class="title[^"]*"[^>]*>(.*?)</div>.*?</a>.*?<div class="content [^"]*"[^>]*>(.*?)</div>"#)
            .expect("valid brave search regex")
    })
}

fn title_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"(?is)<title[^>]*>(.*?)</title>").expect("valid title regex"))
}

fn script_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(?is)<script\b[^>]*>.*?</script>").expect("valid script regex")
    })
}

fn style_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"(?is)<style\b[^>]*>.*?</style>").expect("valid style regex"))
}

fn comment_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"(?is)<!--.*?-->").expect("valid comment regex"))
}

fn tag_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"(?is)<[^>]+>").expect("valid tag regex"))
}

fn whitespace_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"[ \t\x0B\x0C\r]+").expect("valid whitespace regex"))
}

fn blank_line_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"\n{3,}").expect("valid blank line regex"))
}

#[cfg(test)]
mod tests {
    use super::domain_allowed;
    use super::extract_readable_text_from_html;
    use super::normalize_web_fetch_url;
    use super::parse_brave_search_results;
    use pretty_assertions::assert_eq;

    #[test]
    fn parse_brave_search_results_extracts_public_results() {
        let html = r#"
            <div class="snippet  svelte-jmfu5f" data-pos="2" data-type="web" data-keynav="true">
              <div class="result-wrapper">
                <div class="result-content">
                  <a href="https://github.com/openinterpreter/open-interpreter" target="_self">
                    <div class="title search-snippet-title">GitHub - openinterpreter/open-interpreter</div>
                  </a>
                  <div class="generic-snippet">
                    <div class="content desktop-default-regular t-primary line-clamp-dynamic">A natural language interface for computers.</div>
                  </div>
                </div>
              </div>
            </div>
        "#;

        let results = parse_brave_search_results(html, None, None);
        assert_eq!(
            results,
            vec![super::SearchResult {
                title: "GitHub - openinterpreter/open-interpreter".to_string(),
                url: "https://github.com/openinterpreter/open-interpreter".to_string(),
                snippet: "A natural language interface for computers.".to_string(),
            }]
        );
    }

    #[test]
    fn parse_brave_search_results_honors_domain_filters() {
        let html = r#"
            <div class="snippet" data-type="web">
              <a href="https://github.com/openinterpreter/open-interpreter"><div class="title">GitHub</div></a>
              <div class="content x">GitHub result</div>
            </div>
            <div class="snippet" data-type="web">
              <a href="https://example.com/post"><div class="title">Example</div></a>
              <div class="content x">Example result</div>
            </div>
        "#;

        let allowed = vec!["github.com".to_string()];
        let blocked = vec!["example.com".to_string()];
        let results = parse_brave_search_results(html, Some(&allowed), Some(&blocked));
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].url,
            "https://github.com/openinterpreter/open-interpreter"
        );
    }

    #[test]
    fn extract_readable_text_from_html_strips_markup() {
        let html = r#"
            <html>
              <head>
                <title>Example Page</title>
                <style>.hidden { display: none; }</style>
              </head>
              <body>
                <script>alert("ignore")</script>
                <p>Hello <strong>world</strong>.</p>
                <div>Second line &amp; details.</div>
              </body>
            </html>
        "#;

        assert_eq!(
            extract_readable_text_from_html(html),
            "Title: Example Page\n\nHello world.\nSecond line & details."
        );
    }

    #[test]
    fn normalize_web_fetch_url_upgrades_http() {
        let url =
            normalize_web_fetch_url("http://example.com/docs").expect("http URL should upgrade");
        assert_eq!(url.as_str(), "https://example.com/docs");
    }

    #[test]
    fn domain_allowed_blocks_and_allows_suffix_matches() {
        let allowed = vec!["example.com".to_string()];
        let blocked = vec!["blocked.example.com".to_string()];
        assert!(domain_allowed("www.example.com", Some(&allowed), None));
        assert!(!domain_allowed(
            "blocked.example.com",
            Some(&allowed),
            Some(&blocked)
        ));
    }
}
