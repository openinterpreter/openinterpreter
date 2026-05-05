use crate::AdditionalProperties;
use crate::JsonSchema;
use crate::JsonSchemaPrimitiveType;
use crate::JsonSchemaType;
use crate::ResponsesApiTool;
use crate::ToolSpec;
use crate::claude_code_tool_descriptions::CLAUDE_CODE_BASH_DESCRIPTION;
use crate::claude_code_tool_descriptions::CLAUDE_CODE_GREP_DESCRIPTION;
use crate::claude_code_tool_descriptions::CLAUDE_CODE_READ_DESCRIPTION;
use crate::claude_code_tool_descriptions::CLAUDE_CODE_TODO_WRITE_DESCRIPTION;
use std::collections::BTreeMap;

pub fn create_claude_code_agent_tool() -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: "Agent".to_string(),
        description: "Launch a new agent to handle complex, multi-step tasks. Each agent type has specific capabilities and tools available to it.\n\nAvailable agent types and the tools they have access to:\n- Explore: Fast agent specialized for exploring codebases. Use this when you need to quickly find files by patterns (eg. \"src/components/**/*.tsx\"), search code for keywords (eg. \"API endpoints\"), or answer questions about the codebase (eg. \"how do API endpoints work?\"). When calling this agent, specify the desired thoroughness level: \"quick\" for basic searches, \"medium\" for moderate exploration, or \"very thorough\" for comprehensive analysis across multiple locations and naming conventions. (Tools: All tools except Agent, ExitPlanMode, Edit, Write, NotebookEdit)\n- general-purpose: General-purpose agent for researching complex questions, searching for code, and executing multi-step tasks. When you are searching for a keyword or file and are not confident that you will find the right match in the first few tries use this agent to perform the search for you. (Tools: *)\n- Plan: Software architect agent for designing implementation plans. Use this when you need to plan the implementation strategy for a task. Returns step-by-step plans, identifies critical files, and considers architectural trade-offs. (Tools: All tools except Agent, ExitPlanMode, Edit, Write, NotebookEdit)\n- statusline-setup: Use this agent to configure the user's Claude Code status line setting. (Tools: Read, Edit)\n\nWhen using the Agent tool, specify a subagent_type parameter to select which agent type to use. If omitted, the general-purpose agent is used.\n\n## When not to use\n\nIf the target is already known, use the direct tool: Read for a known path, the Grep tool for a specific symbol or string. Reserve this tool for open-ended questions that span the codebase, or tasks that match an available agent type.\n\n## Usage notes\n\n- Always include a short description summarizing what the agent will do\n- When you launch multiple agents for independent work, send them in a single message with multiple tool uses so they run concurrently\n- When the agent is done, it will return a single message back to you. The result returned by the agent is not visible to the user. To show the user the result, you should send a text message back to the user with a concise summary of the result.\n- Trust but verify: an agent's summary describes what it intended to do, not necessarily what it did. When an agent writes or edits code, check the actual changes before reporting the work as done.\n- You can optionally run agents in the background using the run_in_background parameter. When an agent runs in the background, you will be automatically notified when it completes — do NOT sleep, poll, or proactively check on its progress. Continue with other work or respond to the user instead.\n- **Foreground vs background**: Use foreground (default) when you need the agent's results before you can proceed — e.g., research agents whose findings inform your next steps. Use background when you have genuinely independent work to do in parallel.\n- To continue a previously spawned agent, use SendMessage with the agent's ID or name as the `to` field — that resumes it with full context. A new Agent call starts a fresh agent with no memory of prior runs, so the prompt must be self-contained.\n- Clearly tell the agent whether you expect it to write code or just to do research (search, file reads, web fetches, etc.), since it is not aware of the user's intent\n- If the agent description mentions that it should be used proactively, then you should try your best to use it without the user having to ask for it first.\n- If the user specifies that they want you to run agents \"in parallel\", you MUST send a single message with multiple Agent tool use content blocks. For example, if you need to launch both a build-validator agent and a test-runner agent in parallel, send a single message with both tool calls.\n- With `isolation: \"worktree\"`, the worktree is automatically cleaned up if the agent makes no changes; otherwise the path and branch are returned in the result.\n\n## Writing the prompt\n\nBrief the agent like a smart colleague who just walked into the room — it hasn't seen this conversation, doesn't know what you've tried, doesn't understand why this task matters.\n- Explain what you're trying to accomplish and why.\n- Describe what you've already learned or ruled out.\n- Give enough context about the surrounding problem that the agent can make judgment calls rather than just following a narrow instruction.\n- If you need a short response, say so (\"report in under 200 words\").\n- Lookups: hand over the exact command. Investigations: hand over the question — prescribed steps become dead weight when the premise is wrong.\n\nTerse command-style prompts produce shallow, generic work.\n\n**Never delegate understanding.** Don't write \"based on your findings, fix the bug\" or \"based on the research, implement it.\" Those phrases push synthesis onto the agent instead of doing it yourself. Write prompts that prove you understood: include file paths, line numbers, what specifically to change.\n\nExample usage:\n\n<example>\nuser: \"What's left on this branch before we can ship?\"\nassistant: <thinking>A survey question across git state, tests, and config. I'll delegate it and ask for a short report so the raw command output stays out of my context.</thinking>\nAgent({\n  description: \"Branch ship-readiness audit\",\n  prompt: \"Audit what's left before this branch can ship. Check: uncommitted changes, commits ahead of main, whether tests exist, whether the GrowthBook gate is wired up, whether CI-relevant files changed. Report a punch list — done vs. missing. Under 200 words.\"\n})\n<commentary>\nThe prompt is self-contained: it states the goal, lists what to check, and caps the response length. The agent's report comes back as the tool result; relay the findings to the user.\n</commentary>\n</example>\n\n<example>\nuser: \"Can you get a second opinion on whether this migration is safe?\"\nassistant: <thinking>I'll ask the code-reviewer agent — it won't see my analysis, so it can give an independent read.</thinking>\nAgent({\n  description: \"Independent migration review\",\n  subagent_type: \"code-reviewer\",\n  prompt: \"Review migration 0042_user_schema.sql for safety. Context: we're adding a NOT NULL column to a 50M-row table. Existing rows get a backfill default. I want a second opinion on whether the backfill approach is safe under concurrent writes — I've checked locking behavior but want independent verification. Report: is this safe, and if not, what specifically breaks?\"\n})\n<commentary>\nThe agent starts with no context from this conversation, so the prompt briefs it: what to assess, the relevant background, and what form the answer should take.\n</commentary>\n</example>\n"
            .to_string(),
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
                (
                    "subagent_type".to_string(),
                    JsonSchema::string(Some(
                        "The type of specialized agent to use for this task".to_string(),
                    )),
                ),
                (
                    "model".to_string(),
                    JsonSchema::string_enum(
                        vec![
                            "sonnet".into(),
                            "opus".into(),
                            "haiku".into(),
                        ],
                        Some(
                            "Optional model override for this agent. Takes precedence over the agent definition's model frontmatter. If omitted, uses the agent definition's model, or inherits from the parent."
                                .to_string(),
                        ),
                    ),
                ),
                (
                    "run_in_background".to_string(),
                    JsonSchema::boolean(Some(
                        "Set to true to run this agent in the background. You will be notified when it completes."
                            .to_string(),
                    )),
                ),
                (
                    "isolation".to_string(),
                    JsonSchema::string_enum(
                        vec!["worktree".into()],
                        Some(
                            "Isolation mode. \"worktree\" creates a temporary git worktree so the agent works on an isolated copy of the repo."
                                .to_string(),
                        ),
                    ),
                ),
            ]),
            Some(vec!["description".to_string(), "prompt".to_string()]),
            Some(false.into()),
        ),
        output_schema: None,
    })
}

pub fn create_claude_code_read_tool() -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: "Read".to_string(),
        description: CLAUDE_CODE_READ_DESCRIPTION.to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            BTreeMap::from([
                (
                    "file_path".to_string(),
                    JsonSchema::string(Some(
                        "The absolute path to the file to read".to_string(),
                    )),
                ),
                (
                    "offset".to_string(),
                    JsonSchema::number(Some(
                        "The line number to start reading from. Only provide if the file is too large to read at once"
                            .to_string(),
                    )),
                ),
                (
                    "limit".to_string(),
                    JsonSchema::number(Some(
                        "The number of lines to read. Only provide if the file is too large to read at once."
                            .to_string(),
                    )),
                ),
                (
                    "pages".to_string(),
                    JsonSchema::string(Some(
                        "Page range for PDF files (e.g., \"1-5\", \"3\", \"10-20\"). Only applicable to PDF files. Maximum 20 pages per request."
                            .to_string(),
                    )),
                ),
            ]),
            Some(vec!["file_path".to_string()]),
            Some(false.into()),
        ),
        output_schema: None,
    })
}

pub fn create_claude_code_glob_tool() -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: "Glob".to_string(),
        description: "- Fast file pattern matching tool that works with any codebase size\n- Supports glob patterns like \"**/*.js\" or \"src/**/*.ts\"\n- Returns matching file paths sorted by modification time\n- Use this tool when you need to find files by name patterns\n- When you are doing an open ended search that may require multiple rounds of globbing and grepping, use the Agent tool instead"
            .to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            BTreeMap::from([
                (
                    "pattern".to_string(),
                    JsonSchema::string(Some(
                        "The glob pattern to match files against".to_string(),
                    )),
                ),
                (
                    "path".to_string(),
                    JsonSchema::string(Some(
                        "The directory to search in. If not specified, the current working directory will be used. IMPORTANT: Omit this field to use the default directory. DO NOT enter \"undefined\" or \"null\" - simply omit it for the default behavior. Must be a valid directory path if provided."
                            .to_string(),
                    )),
                ),
            ]),
            Some(vec!["pattern".to_string()]),
            Some(false.into()),
        ),
        output_schema: None,
    })
}

pub fn create_claude_code_grep_tool() -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: "Grep".to_string(),
        description: CLAUDE_CODE_GREP_DESCRIPTION.to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            BTreeMap::from([
                (
                    "pattern".to_string(),
                    JsonSchema::string(Some(
                        "The regular expression pattern to search for in file contents"
                            .to_string(),
                    )),
                ),
                (
                    "path".to_string(),
                    JsonSchema::string(Some(
                        "File or directory to search in (rg PATH). Defaults to current working directory."
                            .to_string(),
                    )),
                ),
                (
                    "glob".to_string(),
                    JsonSchema::string(Some(
                        "Glob pattern to filter files (e.g. \"*.js\", \"*.{ts,tsx}\") - maps to rg --glob"
                            .to_string(),
                    )),
                ),
                (
                    "output_mode".to_string(),
                    JsonSchema::string_enum(
                        vec![
                            "content".into(),
                            "files_with_matches".into(),
                            "count".into(),
                        ],
                        Some(
                            "Output mode: \"content\" shows matching lines (supports -A/-B/-C context, -n line numbers, head_limit), \"files_with_matches\" shows file paths (supports head_limit), \"count\" shows match counts (supports head_limit). Defaults to \"files_with_matches\"."
                                .to_string(),
                        ),
                    ),
                ),
                (
                    "-B".to_string(),
                    JsonSchema::number(Some(
                        "Number of lines to show before each match (rg -B). Requires output_mode: \"content\", ignored otherwise."
                            .to_string(),
                    )),
                ),
                (
                    "-A".to_string(),
                    JsonSchema::number(Some(
                        "Number of lines to show after each match (rg -A). Requires output_mode: \"content\", ignored otherwise."
                            .to_string(),
                    )),
                ),
                (
                    "-C".to_string(),
                    JsonSchema::number(Some("Alias for context.".to_string())),
                ),
                (
                    "context".to_string(),
                    JsonSchema::number(Some(
                        "Number of lines to show before and after each match (rg -C). Requires output_mode: \"content\", ignored otherwise."
                            .to_string(),
                    )),
                ),
                (
                    "-n".to_string(),
                    JsonSchema::boolean(Some(
                        "Show line numbers in output (rg -n). Requires output_mode: \"content\", ignored otherwise. Defaults to true."
                            .to_string(),
                    )),
                ),
                (
                    "-i".to_string(),
                    JsonSchema::boolean(Some("Case insensitive search (rg -i)".to_string())),
                ),
                (
                    "type".to_string(),
                    JsonSchema::string(Some(
                        "File type to search (rg --type). Common types: js, py, rust, go, java, etc. More efficient than include for standard file types."
                            .to_string(),
                    )),
                ),
                (
                    "head_limit".to_string(),
                    JsonSchema::number(Some(
                        "Limit output to first N lines/entries, equivalent to \"| head -N\". Works across all output modes: content (limits output lines), files_with_matches (limits file paths), count (limits count entries). Defaults to 250 when unspecified. Pass 0 for unlimited (use sparingly — large result sets waste context)."
                            .to_string(),
                    )),
                ),
                (
                    "offset".to_string(),
                    JsonSchema::number(Some(
                        "Skip first N lines/entries before applying head_limit, equivalent to \"| tail -n +N | head -N\". Works across all output modes. Defaults to 0."
                            .to_string(),
                    )),
                ),
                (
                    "multiline".to_string(),
                    JsonSchema::boolean(Some(
                        "Enable multiline mode where . matches newlines and patterns can span lines (rg -U --multiline-dotall). Default: false."
                            .to_string(),
                    )),
                ),
            ]),
            Some(vec!["pattern".to_string()]),
            Some(false.into()),
        ),
        output_schema: None,
    })
}

pub fn create_claude_code_lsp_tool() -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: "LSP".to_string(),
        description: "Interact with Language Server Protocol (LSP) servers to get code intelligence features.\n\nSupported operations:\n- goToDefinition: Find where a symbol is defined\n- findReferences: Find all references to a symbol\n- hover: Get hover information (documentation, type info) for a symbol\n- documentSymbol: Get all symbols (functions, classes, variables) in a document\n- workspaceSymbol: Search for symbols across the entire workspace\n- goToImplementation: Find implementations of an interface or abstract method\n- prepareCallHierarchy: Get call hierarchy item at a position (functions/methods)\n- incomingCalls: Find all functions/methods that call the function at a position\n- outgoingCalls: Find all functions/methods called by the function at a position\n\nAll operations require:\n- filePath: The file to operate on\n- line: The line number (1-based, as shown in editors)\n- character: The character offset (1-based, as shown in editors)\n\nNote: LSP servers must be configured for the file type. If no server is available, an error will be returned."
            .to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            BTreeMap::from([
                (
                    "operation".to_string(),
                    JsonSchema::string_enum(
                        vec![
                            "goToDefinition".into(),
                            "findReferences".into(),
                            "hover".into(),
                            "documentSymbol".into(),
                            "workspaceSymbol".into(),
                            "goToImplementation".into(),
                            "prepareCallHierarchy".into(),
                            "incomingCalls".into(),
                            "outgoingCalls".into(),
                        ],
                        Some("The LSP operation to perform".to_string()),
                    ),
                ),
                (
                    "filePath".to_string(),
                    JsonSchema::string(Some(
                        "The absolute or relative path to the file".to_string(),
                    )),
                ),
                (
                    "line".to_string(),
                    JsonSchema::integer(Some(
                        "The line number (1-based, as shown in editors)".to_string(),
                    )),
                ),
                (
                    "character".to_string(),
                    JsonSchema::integer(Some(
                        "The character offset (1-based, as shown in editors)".to_string(),
                    )),
                ),
            ]),
            Some(vec![
                "operation".to_string(),
                "filePath".to_string(),
                "line".to_string(),
                "character".to_string(),
            ]),
            Some(false.into()),
        ),
        output_schema: None,
    })
}

#[cfg(test)]
#[path = "claude_code_tool_tests.rs"]
mod tests;

pub fn create_claude_code_write_tool() -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: "Write".to_string(),
        description: "Writes a file to the local filesystem.\n\nUsage:\n- This tool will overwrite the existing file if there is one at the provided path.\n- If this is an existing file, you MUST use the Read tool first to read the file's contents. This tool will fail if you did not read the file first.\n- Prefer the Edit tool for modifying existing files — it only sends the diff. Only use this tool to create new files or for complete rewrites.\n- NEVER create documentation files (*.md) or README files unless explicitly requested by the User.\n- Only use emojis if the user explicitly requests it. Avoid writing emojis to files unless asked."
            .to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            BTreeMap::from([
                (
                    "file_path".to_string(),
                    JsonSchema::string(Some(
                        "The absolute path to the file to write (must be absolute, not relative)"
                            .to_string(),
                    )),
                ),
                (
                    "content".to_string(),
                    JsonSchema::string(Some("The content to write to the file".to_string())),
                ),
            ]),
            Some(vec!["file_path".to_string(), "content".to_string()]),
            Some(false.into()),
        ),
        output_schema: None,
    })
}

pub fn create_claude_code_todo_write_tool() -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: "TodoWrite".to_string(),
        description: CLAUDE_CODE_TODO_WRITE_DESCRIPTION.to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            BTreeMap::from([(
                "todos".to_string(),
                JsonSchema::array(
                    JsonSchema::object(
                        BTreeMap::from([
                            (
                                "content".to_string(),
                                JsonSchema::string(Some(
                                    "The task description in imperative form".to_string(),
                                )),
                            ),
                            (
                                "status".to_string(),
                                JsonSchema::string(Some(
                                    "One of: pending, in_progress, completed".to_string(),
                                )),
                            ),
                            (
                                "activeForm".to_string(),
                                JsonSchema::string(Some(
                                    "The task description in present continuous form".to_string(),
                                )),
                            ),
                        ]),
                        Some(vec![
                            "content".to_string(),
                            "status".to_string(),
                            "activeForm".to_string(),
                        ]),
                        Some(false.into()),
                    ),
                    Some("The updated todo list".to_string()),
                ),
            )]),
            Some(vec!["todos".to_string()]),
            Some(false.into()),
        ),
        output_schema: None,
    })
}

pub fn create_claude_code_ask_user_question_tool() -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: "AskUserQuestion".to_string(),
        description: "Use this tool when you need to ask the user questions during execution. This allows you to:\n1. Gather user preferences or requirements\n2. Clarify ambiguous instructions\n3. Get decisions on implementation choices as you work\n4. Offer choices to the user about what direction to take.\n\nUsage notes:\n- Users will always be able to select \"Other\" to provide custom text input\n- Use multiSelect: true to allow multiple answers to be selected for a question\n- If you recommend a specific option, make that the first option in the list and add \"(Recommended)\" at the end of the label\n\nPlan mode note: In plan mode, use this tool to clarify requirements or choose between approaches BEFORE finalizing your plan. Do NOT use this tool to ask \"Is my plan ready?\" or \"Should I proceed?\" - use ExitPlanMode for plan approval. IMPORTANT: Do not reference \"the plan\" in your questions (e.g., \"Do you have feedback about the plan?\", \"Does the plan look good?\") because the user cannot see the plan in the UI until you call ExitPlanMode. If you need plan approval, use ExitPlanMode instead.\n"
            .to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            BTreeMap::from([
                (
                    "questions".to_string(),
                    JsonSchema {
                        schema_type: Some(JsonSchemaType::Single(JsonSchemaPrimitiveType::Array)),
                        description: Some("Questions to ask the user (1-4 questions)".to_string()),
                        items: Some(Box::new(JsonSchema::object(
                            BTreeMap::from([
                                (
                                    "question".to_string(),
                                    JsonSchema::string(Some(
                                        "The complete question to ask the user. Should be clear, specific, and end with a question mark. Example: \"Which library should we use for date formatting?\" If multiSelect is true, phrase it accordingly, e.g. \"Which features do you want to enable?\"".to_string(),
                                    )),
                                ),
                                (
                                    "header".to_string(),
                                    JsonSchema::string(Some(
                                        "Very short label displayed as a chip/tag (max 12 chars). Examples: \"Auth method\", \"Library\", \"Approach\".".to_string(),
                                    )),
                                ),
                                (
                                    "options".to_string(),
                                    JsonSchema {
                                        schema_type: Some(JsonSchemaType::Single(
                                            JsonSchemaPrimitiveType::Array,
                                        )),
                                        description: Some(
                                            "The available choices for this question. Must have 2-4 options. Each option should be a distinct, mutually exclusive choice (unless multiSelect is enabled). There should be no 'Other' option, that will be provided automatically."
                                                .to_string(),
                                        ),
                                        items: Some(Box::new(JsonSchema::object(
                                            BTreeMap::from([
                                                (
                                                    "label".to_string(),
                                                    JsonSchema::string(Some(
                                                        "The display text for this option that the user will see and select. Should be concise (1-5 words) and clearly describe the choice.".to_string(),
                                                    )),
                                                ),
                                                (
                                                    "description".to_string(),
                                                    JsonSchema::string(Some(
                                                        "Explanation of what this option means or what will happen if chosen. Useful for providing context about trade-offs or implications.".to_string(),
                                                    )),
                                                ),
                                                (
                                                    "preview".to_string(),
                                                    JsonSchema::string(Some(
                                                        "Optional preview content rendered when this option is focused. Use for mockups, code snippets, or visual comparisons that help users compare options. See the tool description for the expected content format.".to_string(),
                                                    )),
                                                ),
                                            ]),
                                            Some(vec![
                                                "label".to_string(),
                                                "description".to_string(),
                                            ]),
                                            Some(false.into()),
                                        ))),
                                        min_items: Some(2),
                                        max_items: Some(4),
                                        ..Default::default()
                                    },
                                ),
                                (
                                    "multiSelect".to_string(),
                                    JsonSchema {
                                        schema_type: Some(JsonSchemaType::Single(
                                            JsonSchemaPrimitiveType::Boolean,
                                        )),
                                        description: Some(
                                            "Set to true to allow the user to select multiple options instead of just one. Use when choices are not mutually exclusive."
                                                .to_string(),
                                        ),
                                        default_value: Some(false.into()),
                                        ..Default::default()
                                    },
                                ),
                            ]),
                            Some(vec![
                                "question".to_string(),
                                "header".to_string(),
                                "options".to_string(),
                                "multiSelect".to_string(),
                            ]),
                            Some(false.into()),
                        ))),
                        min_items: Some(1),
                        max_items: Some(4),
                        ..Default::default()
                    },
                ),
                (
                    "answers".to_string(),
                    JsonSchema {
                        schema_type: Some(JsonSchemaType::Single(JsonSchemaPrimitiveType::Object)),
                        description: Some(
                            "User answers collected by the permission component".to_string(),
                        ),
                        property_names: Some(Box::new(JsonSchema::string(None))),
                        additional_properties: Some(
                            AdditionalProperties::Schema(Box::new(JsonSchema::string(None))),
                        ),
                        ..Default::default()
                    },
                ),
                (
                    "annotations".to_string(),
                    JsonSchema {
                        schema_type: Some(JsonSchemaType::Single(JsonSchemaPrimitiveType::Object)),
                        description: Some(
                            "Optional per-question annotations from the user (e.g., notes on preview selections). Keyed by question text.".to_string(),
                        ),
                        property_names: Some(Box::new(JsonSchema::string(None))),
                        additional_properties: Some(AdditionalProperties::Schema(Box::new(
                            JsonSchema::object(
                                BTreeMap::from([
                                    (
                                        "preview".to_string(),
                                        JsonSchema::string(Some(
                                            "The preview content of the selected option, if the question used previews.".to_string(),
                                        )),
                                    ),
                                    (
                                        "notes".to_string(),
                                        JsonSchema::string(Some(
                                            "Free-text notes the user added to their selection.".to_string(),
                                        )),
                                    ),
                                ]),
                                None,
                                Some(false.into()),
                            ),
                        ))),
                        ..Default::default()
                    },
                ),
                (
                    "metadata".to_string(),
                    JsonSchema {
                        schema_type: Some(JsonSchemaType::Single(JsonSchemaPrimitiveType::Object)),
                        description: Some(
                            "Optional metadata for tracking and analytics purposes. Not displayed to user.".to_string(),
                        ),
                        properties: Some([(
                            "source".to_string(),
                            JsonSchema::string(Some(
                                "Optional identifier for the source of this question (e.g., \"remember\" for /remember command). Used for analytics tracking.".to_string(),
                            )),
                        )]
                        .into_iter()
                        .collect()),
                        additional_properties: Some(false.into()),
                        ..Default::default()
                    },
                ),
            ]),
            Some(vec!["questions".to_string()]),
            Some(false.into()),
        ),
        output_schema: None,
    })
}

pub fn create_claude_code_edit_tool() -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: "Edit".to_string(),
        description: "Performs exact string replacements in files.\n\nUsage:\n- You must use your `Read` tool at least once in the conversation before editing. This tool will error if you attempt an edit without reading the file.\n- When editing text from Read tool output, ensure you preserve the exact indentation (tabs/spaces) as it appears AFTER the line number prefix. The line number prefix format is: line number + tab. Everything after that is the actual file content to match. Never include any part of the line number prefix in the old_string or new_string.\n- ALWAYS prefer editing existing files in the codebase. NEVER write new files unless explicitly required.\n- Only use emojis if the user explicitly requests it. Avoid adding emojis to files unless asked.\n- The edit will FAIL if `old_string` is not unique in the file. Either provide a larger string with more surrounding context to make it unique or use `replace_all` to change every instance of `old_string`.\n- Use `replace_all` for replacing and renaming strings across the file. This parameter is useful if you want to rename a variable for instance."
            .to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            BTreeMap::from([
                (
                    "file_path".to_string(),
                    JsonSchema::string(Some(
                        "The absolute path to the file to modify".to_string(),
                    )),
                ),
                (
                    "old_string".to_string(),
                    JsonSchema::string(Some("The text to replace".to_string())),
                ),
                (
                    "new_string".to_string(),
                    JsonSchema::string(Some(
                        "The text to replace it with (must be different from old_string)"
                            .to_string(),
                    )),
                ),
                (
                    "replace_all".to_string(),
                    JsonSchema::boolean(Some(
                        "Replace all occurrences of old_string (default false)".to_string(),
                    )),
                ),
            ]),
            Some(vec![
                "file_path".to_string(),
                "old_string".to_string(),
                "new_string".to_string(),
            ]),
            Some(false.into()),
        ),
        output_schema: None,
    })
}

pub fn create_claude_code_bash_tool() -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: "Bash".to_string(),
        description: CLAUDE_CODE_BASH_DESCRIPTION.to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            BTreeMap::from([
                (
                    "command".to_string(),
                    JsonSchema::string(Some("The bash command to execute".to_string())),
                ),
                (
                    "description".to_string(),
                    JsonSchema::string(Some(
                        "A short description of what the command does".to_string(),
                    )),
                ),
                (
                    "timeout".to_string(),
                    JsonSchema::number(Some(
                        "Optional timeout in milliseconds, up to 600000".to_string(),
                    )),
                ),
                (
                    "run_in_background".to_string(),
                    JsonSchema::boolean(Some(
                        "Run the command in the background and return immediately".to_string(),
                    )),
                ),
            ]),
            Some(vec!["command".to_string()]),
            Some(false.into()),
        ),
        output_schema: None,
    })
}
