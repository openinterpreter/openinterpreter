use super::*;
use pretty_assertions::assert_eq;

fn metric_call_tool_result(
    is_error: bool,
    structured_content: Option<serde_json::Value>,
) -> CallToolResult {
    CallToolResult {
        content: Vec::new(),
        structured_content,
        is_error: Some(is_error),
        meta: None,
    }
}

#[test]
fn mcp_call_metric_tags_include_server_name() {
    assert_eq!(
        mcp_call_metric_tags(
            "error",
            "docs server",
            "search docs",
            Some("connector/docs"),
            Some("Docs connector"),
        ),
        vec![
            ("status", "error".to_string()),
            ("server", "docs_server".to_string()),
            ("tool", "search_docs".to_string()),
            ("connector_id", "connector/docs".to_string()),
            ("connector_name", "Docs_connector".to_string()),
        ],
    );
}

#[test]
fn mcp_call_metric_outcome_distinguishes_request_and_tool_errors() {
    assert_eq!(
        mcp_call_metric_outcome(&Ok(metric_call_tool_result(
            /*is_error*/ false, /*structured_content*/ None,
        )),),
        McpCallMetricOutcome {
            status: "ok",
            error_type: None,
            error_code: None,
            approval_source: None,
        }
    );
    assert_eq!(
        mcp_call_metric_outcome(&Ok(metric_call_tool_result(
            /*is_error*/ true,
            Some(serde_json::json!({"error_code": "RATE_LIMITED"})),
        )),),
        McpCallMetricOutcome {
            status: "error",
            error_type: Some(MCP_CALL_ERROR_TYPE_TOOL_RESULT),
            error_code: Some("RATE_LIMITED".to_string()),
            approval_source: None,
        }
    );
    assert_eq!(
        mcp_call_metric_outcome(&Err("connection closed".to_string())),
        McpCallMetricOutcome {
            status: "error",
            error_type: Some(MCP_CALL_ERROR_TYPE_MCP_REQUEST),
            error_code: Some(MCP_CALL_ERROR_CODE_UNKNOWN.to_string()),
            approval_source: None,
        }
    );
}

#[test]
fn mcp_call_metric_outcome_reports_server_tool_error_codes() {
    let result = Ok(metric_call_tool_result(
        /*is_error*/ true,
        Some(serde_json::json!({"error_code": "arbitrary-user-value"})),
    ));

    assert_eq!(
        mcp_call_metric_outcome(&result),
        McpCallMetricOutcome {
            status: "error",
            error_type: Some(MCP_CALL_ERROR_TYPE_TOOL_RESULT),
            error_code: Some("arbitrary-user-value".to_string()),
            approval_source: None,
        }
    );
}

#[test]
fn mcp_call_metric_outcome_reads_auth_error_code_from_meta() {
    let result = CallToolResult {
        content: Vec::new(),
        structured_content: None,
        is_error: Some(true),
        meta: Some(serde_json::json!({
            MCP_TOOL_CODEX_APPS_META_KEY: {
                "connector_auth_failure": {
                    "is_auth_failure": true,
                    "error_code": "UNAUTHORIZED",
                },
            },
        })),
    };

    assert_eq!(
        mcp_call_metric_outcome(&Ok(result)),
        McpCallMetricOutcome {
            status: "error",
            error_type: Some(MCP_CALL_ERROR_TYPE_TOOL_RESULT),
            error_code: Some("UNAUTHORIZED".to_string()),
            approval_source: None,
        }
    );
}

#[test]
fn mcp_call_metric_outcome_reads_telemetry_without_changing_model_output() {
    let mut result = CallToolResult {
        content: vec![
            serde_json::json!({"type": "text", "text": "browser failed"}),
            serde_json::json!({"type": "text", "text": "captured output"}),
        ],
        structured_content: None,
        is_error: Some(true),
        meta: None,
    };
    let original_output = result.as_function_call_output_payload();
    result.meta = Some(serde_json::json!({
        "codex/telemetry": {"error_code": "locator_timeout"},
    }));

    assert_eq!(result.as_function_call_output_payload(), original_output);
    assert_eq!(
        mcp_call_metric_outcome(&Ok(result)),
        McpCallMetricOutcome {
            status: "error",
            error_type: Some(MCP_CALL_ERROR_TYPE_TOOL_RESULT),
            error_code: Some("locator_timeout".to_string()),
            approval_source: None,
        }
    );
}

#[test]
fn mcp_call_metric_outcome_preserves_existing_error_code_precedence() {
    let mut result = metric_call_tool_result(
        /*is_error*/ true,
        Some(serde_json::json!({"error_code": "RATE_LIMITED"})),
    );
    result.meta = Some(serde_json::json!({
        "codex/telemetry": {"error_code": "locator_timeout"},
        MCP_TOOL_CODEX_APPS_META_KEY: {
            "connector_auth_failure": {
                "is_auth_failure": true,
                "error_code": "UNAUTHORIZED",
            },
        },
    }));
    for (structured_content, expected_code) in [
        (result.structured_content.clone(), "RATE_LIMITED"),
        (None, "UNAUTHORIZED"),
    ] {
        result.structured_content = structured_content;
        assert_eq!(
            mcp_call_metric_outcome(&Ok(result.clone())),
            McpCallMetricOutcome {
                status: "error",
                error_type: Some(MCP_CALL_ERROR_TYPE_TOOL_RESULT),
                error_code: Some(expected_code.to_string()),
                approval_source: None,
            }
        );
    }
}

#[test]
fn mcp_call_metric_outcome_ignores_invalid_telemetry_codes() {
    for metadata in [
        serde_json::json!({"codex/telemetry": "invalid"}),
        serde_json::json!({"codex/telemetry": {"error_code": 42}}),
        serde_json::json!({"codex/telemetry": {"error_code": "Bearer credential-sentinel"}}),
        serde_json::json!({"codex/telemetry": {"error_code": "locator_timeout:secret-sentinel"}}),
    ] {
        let mut result = metric_call_tool_result(
            /*is_error*/ true,
            Some(serde_json::json!({"error_code": ""})),
        );
        result.meta = Some(metadata);
        assert_eq!(
            mcp_call_metric_outcome(&Ok(result)),
            McpCallMetricOutcome {
                status: "error",
                error_type: Some(MCP_CALL_ERROR_TYPE_TOOL_RESULT),
                error_code: Some(MCP_CALL_ERROR_CODE_UNKNOWN.to_string()),
                approval_source: None,
            }
        );
    }
}

#[test]
fn mcp_call_metric_outcome_bounds_and_sanitizes_error_code() {
    let raw_error_code = format!("BAD CODE {}", "x".repeat(300));
    let result = metric_call_tool_result(
        /*is_error*/ true,
        Some(serde_json::json!({"error_code": raw_error_code})),
    );

    assert_eq!(
        mcp_call_metric_outcome(&Ok(result)),
        McpCallMetricOutcome {
            status: "error",
            error_type: Some(MCP_CALL_ERROR_TYPE_TOOL_RESULT),
            error_code: Some(format!("BAD_CODE_{}", "x".repeat(247))),
            approval_source: None,
        }
    );
}

#[test]
fn caller_error_code_only_applies_to_request_failures() {
    assert_eq!(
        mcp_call_metric_outcome(&Err("deadline".to_string()))
            .with_request_error_code(Some("request_timeout")),
        McpCallMetricOutcome {
            status: "error",
            error_type: Some(MCP_CALL_ERROR_TYPE_MCP_REQUEST),
            error_code: Some("request_timeout".to_string()),
            approval_source: None,
        }
    );
    let result = Ok(metric_call_tool_result(
        /*is_error*/ true,
        Some(serde_json::json!({"error_code": "locator_timeout"})),
    ));
    assert_eq!(
        mcp_call_metric_outcome(&result).with_request_error_code(Some("request_timeout")),
        mcp_call_metric_outcome(&result)
    );
}
