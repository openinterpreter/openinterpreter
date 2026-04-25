use crate::JsonSchema;
use crate::ResponsesApiTool;
use crate::ToolSpec;
use std::collections::BTreeMap;

const CLAUDE_CODE_WEB_FETCH_DESCRIPTION: &str = "IMPORTANT: WebFetch WILL FAIL for authenticated or private URLs. Before using this tool, check if the URL points to an authenticated service (e.g. Google Docs, Confluence, Jira, GitHub). If so, look for a specialized MCP tool that provides authenticated access.\n\n- Fetches content from a specified URL and processes it using an AI model\n- Takes a URL and a prompt as input\n- Fetches the URL content, converts HTML to markdown\n- Processes the content with the prompt using a small, fast model\n- Returns the model's response about the content\n- Use this tool when you need to retrieve and analyze web content\n\nUsage notes:\n  - IMPORTANT: If an MCP-provided web fetch tool is available, prefer using that tool instead of this one, as it may have fewer restrictions.\n  - The URL must be a fully-formed valid URL\n  - HTTP URLs will be automatically upgraded to HTTPS\n  - The prompt should describe what information you want to extract from the page\n  - This tool is read-only and does not modify any files\n  - Results may be summarized if the content is very large\n  - Includes a self-cleaning 15-minute cache for faster responses when repeatedly accessing the same URL\n  - When a URL redirects to a different host, the tool will inform you and provide the redirect URL in a special format. You should then make a new WebFetch request with the redirect URL to fetch the content.\n  - For GitHub URLs, prefer using the gh CLI via Bash instead (e.g., gh pr view, gh issue view, gh api).\n";

const CLAUDE_CODE_WEB_SEARCH_DESCRIPTION: &str = "\n- Allows Claude to search the web and use the results to inform responses\n- Provides up-to-date information for current events and recent data\n- Returns search result information formatted as search result blocks, including links as markdown hyperlinks\n- Use this tool for accessing information beyond Claude's knowledge cutoff\n- Searches are performed automatically within a single API call\n\nCRITICAL REQUIREMENT - You MUST follow this:\n  - After answering the user's question, you MUST include a \"Sources:\" section at the end of your response\n  - In the Sources section, list all relevant URLs from the search results as markdown hyperlinks: [Title](URL)\n  - This is MANDATORY - never skip including sources in your response\n  - Example format:\n\n    [Your answer here]\n\n    Sources:\n    - [Source Title 1](https://example.com/1)\n    - [Source Title 2](https://example.com/2)\n\nUsage notes:\n  - Domain filtering is supported to include or block specific websites\n  - Web search is only available in the US\n\nIMPORTANT - Use the correct year in search queries:\n  - The current month is April 2026. You MUST use this year when searching for recent information, documentation, or current events.\n  - Example: If the user asks for \"latest React docs\", search for \"React documentation\" with the current year, NOT last year\n";

pub fn create_claude_code_web_fetch_tool() -> ToolSpec {
    let mut url_schema = JsonSchema::string(Some("The URL to fetch content from".to_string()));
    url_schema.format = Some("uri".to_string());
    ToolSpec::Function(ResponsesApiTool {
        name: "WebFetch".to_string(),
        description: CLAUDE_CODE_WEB_FETCH_DESCRIPTION.to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            BTreeMap::from([
                ("url".to_string(), url_schema),
                (
                    "prompt".to_string(),
                    JsonSchema::string(Some(
                        "The prompt to run on the fetched content".to_string(),
                    )),
                ),
            ]),
            Some(vec!["url".to_string(), "prompt".to_string()]),
            Some(false.into()),
        ),
        output_schema: None,
    })
}

pub fn create_claude_code_web_search_tool() -> ToolSpec {
    let mut query_schema = JsonSchema::string(Some("The search query to use".to_string()));
    query_schema.min_length = Some(2);
    ToolSpec::Function(ResponsesApiTool {
        name: "WebSearch".to_string(),
        description: CLAUDE_CODE_WEB_SEARCH_DESCRIPTION.to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            BTreeMap::from([
                ("query".to_string(), query_schema),
                (
                    "allowed_domains".to_string(),
                    JsonSchema::array(
                        JsonSchema::string(None),
                        Some("Only include search results from these domains".to_string()),
                    ),
                ),
                (
                    "blocked_domains".to_string(),
                    JsonSchema::array(
                        JsonSchema::string(None),
                        Some("Never include search results from these domains".to_string()),
                    ),
                ),
            ]),
            Some(vec!["query".to_string()]),
            Some(false.into()),
        ),
        output_schema: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn web_fetch_tool_uses_uri_format_for_url() {
        let ToolSpec::Function(tool) = create_claude_code_web_fetch_tool() else {
            panic!("expected function tool");
        };
        let schema = tool
            .parameters
            .properties
            .expect("properties should be present");
        let url = schema.get("url").expect("url schema should be present");
        assert_eq!(url.format.as_deref(), Some("uri"));
    }

    #[test]
    fn web_search_tool_requires_two_character_queries() {
        let ToolSpec::Function(tool) = create_claude_code_web_search_tool() else {
            panic!("expected function tool");
        };
        let schema = tool
            .parameters
            .properties
            .expect("properties should be present");
        let query = schema.get("query").expect("query schema should be present");
        assert_eq!(query.min_length, Some(2));
    }
}
