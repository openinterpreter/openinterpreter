use crate::JsonSchema;
use crate::ResponsesApiTool;
use crate::ToolSpec;
use serde_json::json;
use std::collections::BTreeMap;

const KIMI_CLI_AGENT_DESCRIPTION: &str = "Start a subagent instance to work on a focused task.\n\nThe Agent tool can either create a new subagent instance or resume an existing one by `agent_id`.\nEach instance keeps its own context history under the current session, so repeated use of the same\ninstance can preserve previous findings and work.\n\n**Available Built-in Agent Types**\n\n- `coder`: Good at general software engineering tasks. (Tools: Shell, ReadFile, ReadMediaFile, Glob, Grep, WriteFile, StrReplaceFile, SearchWeb, FetchURL, Model: inherit, Background: yes). When to use: Use this agent for non-trivial software engineering work that may require reading files, editing code, running commands, and returning a compact but technically complete summary to the parent agent.\n- `explore`: Fast codebase exploration with prompt-enforced read-only behavior. (Tools: Shell, ReadFile, ReadMediaFile, Glob, Grep, SearchWeb, FetchURL, Model: inherit, Background: yes). When to use: Fast agent specialized for exploring codebases. Use this when you need to quickly find files by patterns (e.g. \"src/**/*.yaml\"), search code for keywords (e.g. \"database connection\"), or answer questions about the codebase (e.g. \"how does the auth module work?\"). When calling this agent, specify the desired thoroughness level: \"quick\" for basic searches, \"medium\" for moderate exploration, or \"thorough\" for comprehensive analysis across multiple locations and naming conventions. Use this agent for any read-only exploration that will clearly require more than 3 tool calls. Prefer launching multiple explore agents concurrently when investigating independent questions.\n- `plan`: Read-only implementation planning and architecture design. (Tools: ReadFile, ReadMediaFile, Glob, Grep, SearchWeb, FetchURL, Model: inherit, Background: yes). When to use: Use this agent when the parent agent needs a step-by-step implementation plan, key file identification, and architectural trade-off analysis before code changes are made.\n\n**Usage**\n\n- Always provide a short `description` (3-5 words).\n- Use `subagent_type` to select a built-in agent type. If omitted, `coder` is used.\n- Use `model` when you need to override the built-in type's default model or the parent agent's current model.\n- Use `resume` when you want to continue an existing instance instead of starting a new one.\n- If an existing subagent already has relevant context or the task is a continuation of its prior work, prefer `resume` over creating a new instance.\n- Default to foreground execution. Use `run_in_background=true` only when the task can continue independently, you do not need the result immediately, and there is a clear benefit to returning control before it finishes.\n- Be explicit about whether the subagent should write code or only do research.\n- The subagent result is only visible to you. If the user should see it, summarize it yourself.\n\n**Explore Agent — Preferred for Codebase Research**\n\nWhen you need to understand the codebase before making changes, fixing bugs, or planning features,\nprefer `subagent_type=\"explore\"` over doing the search yourself. The explore agent is optimized for\nfast, read-only codebase investigation. Use it when:\n- Your task will clearly require more than 3 search queries\n- You need to understand how a module, feature, or code path works\n- You are about to enter plan mode and want to gather context first\n- You want to investigate multiple independent questions — launch multiple explore agents concurrently\n\nWhen calling explore, specify the desired thoroughness in the prompt:\n- \"quick\": targeted lookups — find a specific file, function, or config value\n- \"medium\": understand a module — how does auth work, what calls this API\n- \"thorough\": cross-cutting analysis — architecture overview, dependency mapping, multi-module investigation\n\n**When Not To Use Agent**\n\n- Reading a known file path\n- Searching a small number of known files\n- Tasks that can be completed in one or two direct tool calls\n";

pub fn create_kimi_cli_agent_tool() -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: "Agent".to_string(),
        description: KIMI_CLI_AGENT_DESCRIPTION.to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            BTreeMap::from([
                (
                    "description".to_string(),
                    JsonSchema::string(Some(
                        "A short (3-5 word) description of the task".to_string(),
                    )),
                ),
                (
                    "prompt".to_string(),
                    JsonSchema::string(Some("The task for the agent to perform".to_string())),
                ),
                ("subagent_type".to_string(), {
                    let mut schema = JsonSchema::string(Some(
                        "The built-in agent type to use. Defaults to `coder`.".to_string(),
                    ));
                    schema.default_value = Some(json!("coder"));
                    schema
                }),
                ("model".to_string(), {
                    let mut schema = JsonSchema::any_of(
                            vec![JsonSchema::string(None), JsonSchema::null(None)],
                            Some(
                                "Optional model override. Selection priority is: this parameter, then the built-in type default model, then the parent agent's current model."
                                    .to_string(),
                            ),
                        );
                    schema.default_value = Some(json!(null));
                    schema
                }),
                ("resume".to_string(), {
                    let mut schema = JsonSchema::any_of(
                        vec![JsonSchema::string(None), JsonSchema::null(None)],
                        Some(
                            "Optional agent ID to resume instead of creating a new instance."
                                .to_string(),
                        ),
                    );
                    schema.default_value = Some(json!(null));
                    schema
                }),
                ("run_in_background".to_string(), {
                    let mut schema = JsonSchema::boolean(Some(
                            "Whether to run the agent in the background. Prefer false unless the task can continue independently and there is a clear benefit to returning control before the result is needed."
                                .to_string(),
                        ));
                    schema.default_value = Some(json!(false));
                    schema
                }),
                ("timeout".to_string(), {
                    let mut timeout = JsonSchema::integer(None);
                    timeout.minimum = Some(30);
                    timeout.maximum = Some(3_600);
                    let mut schema = JsonSchema::any_of(
                            vec![timeout, JsonSchema::null(None)],
                            Some(
                                "Timeout in seconds for the agent task. Foreground: no default timeout (runs until completion), max 3600s (1hr). Background: default from config (15min), max 3600s (1hr). The agent is stopped if it exceeds this limit."
                                    .to_string(),
                            ),
                        );
                    schema.default_value = Some(json!(null));
                    schema
                }),
            ]),
            Some(vec!["description".to_string(), "prompt".to_string()]),
            None,
        ),
        output_schema: None,
    })
}

#[cfg(test)]
mod tests {
    use super::KIMI_CLI_AGENT_DESCRIPTION;
    use super::create_kimi_cli_agent_tool;
    use crate::AdditionalProperties;
    use crate::ToolSpec;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    #[test]
    fn kimi_agent_tool_matches_captured_description_and_schema_shape() {
        let ToolSpec::Function(tool) = create_kimi_cli_agent_tool() else {
            panic!("expected function tool");
        };

        assert_eq!(tool.description, KIMI_CLI_AGENT_DESCRIPTION);
        assert_eq!(
            tool.parameters.additional_properties,
            None::<AdditionalProperties>
        );
        assert_eq!(
            tool.parameters
                .properties
                .as_ref()
                .and_then(|properties| properties.get("model"))
                .and_then(|schema| schema.default_value.clone()),
            Some(json!(null))
        );
        assert_eq!(
            tool.parameters
                .properties
                .as_ref()
                .and_then(|properties| properties.get("resume"))
                .and_then(|schema| schema.default_value.clone()),
            Some(json!(null))
        );
        let timeout = tool
            .parameters
            .properties
            .as_ref()
            .and_then(|properties| properties.get("timeout"))
            .expect("timeout schema");
        assert_eq!(timeout.default_value, Some(json!(null)));
        assert_eq!(
            timeout
                .any_of
                .as_ref()
                .and_then(|variants| variants.first())
                .and_then(|schema| schema.minimum),
            Some(30)
        );
        assert_eq!(
            timeout
                .any_of
                .as_ref()
                .and_then(|variants| variants.first())
                .and_then(|schema| schema.maximum),
            Some(3_600)
        );
    }
}
