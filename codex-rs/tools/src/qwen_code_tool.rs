use crate::JsonSchema;
use crate::ResponsesApiTool;
use crate::ToolSpec;
use serde_json::json;

fn nullable_string(description: &str) -> JsonSchema {
    let mut schema = JsonSchema::any_of(
        vec![JsonSchema::string(None), JsonSchema::null(None)],
        Some(description.to_string()),
    );
    schema.default_value = Some(json!(null));
    schema
}

pub fn create_qwen_code_read_file_tool() -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: "read_file".to_string(),
        description: "Reads and returns the content of a specified file. If the file is large, the content will be truncated. The tool's response will clearly indicate if truncation has occurred and will provide details on how to read more of the file using the 'offset' and 'limit' parameters. Handles text, images (PNG, JPG, GIF, WEBP, SVG, BMP), PDF files, and Jupyter notebooks (.ipynb). For text files, it can read specific line ranges. For PDF files, use the 'pages' parameter to extract specific page ranges as text (e.g. '1-5'). Max 20 pages per request. This tool can read Jupyter notebooks (.ipynb) and returns structured cell content with outputs.".to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            [
                (
                    "file_path".to_string(),
                    JsonSchema::string(Some("The absolute path to the file to read (e.g., '/home/user/project/file.txt'). Relative paths are not supported. You must provide an absolute path.".to_string())),
                ),
                (
                    "offset".to_string(),
                    JsonSchema::number(Some("Optional: For text files, the 0-based line number to start reading from. Requires 'limit' to be set. Use for paginating through large files.".to_string())),
                ),
                (
                    "limit".to_string(),
                    JsonSchema::number(Some("Optional: For text files, maximum number of lines to read. Use with 'offset' to paginate through large files. If omitted, reads the entire file (if feasible, up to a default limit).".to_string())),
                ),
                (
                    "pages".to_string(),
                    JsonSchema::string(Some("Optional: For PDF files, the page range to extract as text (e.g., '1-5', '3', '10-20'). Pages are 1-indexed. Max 20 pages per request. Open-ended ranges like '3-' are not supported. When provided, PDF content is extracted as text regardless of model capabilities.".to_string())),
                ),
            ],
            Some(vec!["file_path".to_string()]),
            None,
        ),
        output_schema: None,
    })
}

pub fn create_qwen_code_write_file_tool() -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: "write_file".to_string(),
        description: "Write content to a file.".to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            [
                (
                    "file_path".to_string(),
                    JsonSchema::string(Some("Absolute path to the file to write.".to_string())),
                ),
                (
                    "content".to_string(),
                    JsonSchema::string(Some("The complete file content to write.".to_string())),
                ),
            ],
            Some(vec!["file_path".to_string(), "content".to_string()]),
            None,
        ),
        output_schema: None,
    })
}

pub fn create_qwen_code_edit_tool() -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: "edit".to_string(),
        description: "Replaces text within a file. By default, replaces a single occurrence. Set `replace_all` to true when you intend to modify every instance of `old_string`. This tool requires providing significant context around the change to ensure precise targeting. Always use the read_file tool to examine the file's current content before attempting a text replacement.\n\n      The user has the ability to modify the `new_string` content. If modified, this will be stated in the response.\n\nExpectation for required parameters:\n1. `file_path` MUST be an absolute path; otherwise an error will be thrown.\n2. `old_string` MUST be the exact literal text to replace (including all whitespace, indentation, newlines, and surrounding code etc.).\n3. `new_string` MUST be the exact literal text to replace `old_string` with (also including all whitespace, indentation, newlines, and surrounding code etc.). Ensure the resulting code is correct and idiomatic.\n4. NEVER escape `old_string` or `new_string`, that would break the exact literal text requirement.\n**Important:** If ANY of the above are not satisfied, the tool will fail. CRITICAL for `old_string`: Must uniquely identify the single instance to change. Include at least 3 lines of context BEFORE and AFTER the target text, matching whitespace and indentation precisely. If this string matches multiple locations, or does not match exactly, the tool will fail.\n**Multiple replacements:** Set `replace_all` to true when you want to replace every occurrence that matches `old_string`.".to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            [
                (
                    "file_path".to_string(),
                    JsonSchema::string(Some(
                        "The absolute path to the file to modify. Must start with '/'.".to_string(),
                    )),
                ),
                (
                    "old_string".to_string(),
                    JsonSchema::string(Some("The exact literal text to replace, preferably unescaped. For single replacements (default), include at least 3 lines of context BEFORE and AFTER the target text, matching whitespace and indentation precisely. If this string is not the exact literal text (i.e. you escaped it) or does not match exactly, the tool will fail.".to_string())),
                ),
                (
                    "new_string".to_string(),
                    JsonSchema::string(Some("The exact literal text to replace `old_string` with, preferably unescaped. Provide the EXACT text. Ensure the resulting code is correct and idiomatic.".to_string())),
                ),
                (
                    "replace_all".to_string(),
                    JsonSchema::boolean(Some(
                        "Replace all occurrences of old_string (default false).".to_string(),
                    )),
                ),
            ],
            Some(vec![
                "file_path".to_string(),
                "old_string".to_string(),
                "new_string".to_string(),
            ]),
            None,
        ),
        output_schema: None,
    })
}

pub fn create_qwen_code_glob_tool() -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: "glob".to_string(),
        description: "Find files using a glob pattern.".to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            [
                (
                    "pattern".to_string(),
                    JsonSchema::string(Some("Glob pattern to match.".to_string())),
                ),
                (
                    "path".to_string(),
                    nullable_string("Directory to search. Defaults to the working directory."),
                ),
            ],
            Some(vec!["pattern".to_string()]),
            None,
        ),
        output_schema: None,
    })
}

pub fn create_qwen_code_grep_search_tool() -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: "grep_search".to_string(),
        description: "Search file contents with ripgrep-compatible regular expressions."
            .to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            [
                (
                    "pattern".to_string(),
                    JsonSchema::string(Some("Regular expression pattern.".to_string())),
                ),
                (
                    "path".to_string(),
                    nullable_string(
                        "File or directory to search. Defaults to the working directory.",
                    ),
                ),
                (
                    "glob".to_string(),
                    nullable_string("Optional glob pattern to filter files."),
                ),
                ("limit".to_string(), {
                    let mut schema = JsonSchema::integer(Some(
                        "Maximum number of returned lines or paths.".to_string(),
                    ));
                    schema.default_value = Some(json!(250));
                    schema.minimum = Some(0);
                    schema
                }),
            ],
            Some(vec!["pattern".to_string()]),
            None,
        ),
        output_schema: None,
    })
}

pub fn create_qwen_code_shell_tool() -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: "run_shell_command".to_string(),
        description: "Executes a given shell command (as `bash -c <command>`) in a persistent shell session with optional timeout, ensuring proper handling and security measures.\n\nIMPORTANT: This tool is for terminal operations like git, npm, docker, etc. DO NOT use it for file operations (reading, writing, editing, searching, finding files) - use the specialized tools for this instead.\n\n**Usage notes**:\n- The command argument is required.\n- You can specify an optional timeout in milliseconds (up to 600000ms / 10 minutes). If not specified, commands will timeout after 120000ms (2 minutes).\n- It is very helpful if you write a clear, concise description of what this command does in 5-10 words.\n\n- Avoid using run_shell_command with the `find`, `grep`, `cat`, `head`, `tail`, `sed`, `awk`, or `echo` commands, unless explicitly instructed or when these commands are truly necessary for the task. Instead, always prefer using the dedicated tools for these commands:\n  - File search: Use glob (NOT find or ls)\n  - Content search: Use grep_search (NOT grep or rg)\n  - Read files: Use read_file (NOT cat/head/tail)\n  - Edit files: Use edit (NOT sed/awk)\n  - Write files: Use write_file (NOT echo >/cat <<EOF)\n  - Communication: Output text directly (NOT echo/printf)\n- **Shell argument quoting and special characters**: When passing arguments that contain special characters (parentheses `()`, backticks ````, dollar signs `$`, backslashes `\\`, semicolons `;`, pipes `|`, angle brackets `<>`, ampersands `&`, exclamation marks `!`, etc.), you MUST ensure they are properly quoted to prevent the shell from misinterpreting them as shell syntax:\n  - **Single quotes** `'...'` pass everything literally, but cannot contain a literal single quote.\n  - **ANSI-C quoting** `$'...'` supports escape sequences (e.g. `\\n` for newline, `\\'` for single quote) and is the safest approach for multi-line strings or strings with single quotes.\n  - **Heredoc** is the most robust approach for large, multi-line text with mixed quotes:\n    ```bash\n    gh pr create --title \"My Title\" --body \"$(cat <<'HEREDOC'\n    Multi-line body with (parentheses), `backticks`, and 'single-quotes'.\n    HEREDOC\n    )\"\n    ```\n  - NEVER use unescaped single quotes inside single-quoted strings (e.g. `'it\\'s'` is wrong; use `$'it\\'s'` or `\"it's\"` instead).\n  - If unsure, prefer double-quoting arguments and escape inner double-quotes as `\\\"`.\n- When issuing multiple commands:\n  - If the commands are independent and can run in parallel, make multiple run_shell_command tool calls in a single message. For example, if you need to run \"git status\" and \"git diff\", send a single message with two run_shell_command tool calls in parallel.\n  - If the commands depend on each other and must run sequentially, use a single run_shell_command call with '&&' to chain them together (e.g., `git add . && git commit -m \"message\" && git push`). For instance, if one operation must complete before another starts (like mkdir before cp, Write before run_shell_command for git operations, or git add before git commit), run these operations sequentially instead.\n  - Use ';' only when you need to run commands sequentially but don't care if earlier commands fail\n  - DO NOT use newlines to separate commands (newlines are ok in quoted strings)\n- Try to maintain your current working directory throughout the session by using absolute paths and avoiding usage of `cd`. You may use `cd` if the User explicitly requests it.\n  <good-example>\n  pytest /foo/bar/tests\n  </good-example>\n  <bad-example>\n  cd /foo/bar && pytest tests\n  </bad-example>\n\n**Background vs Foreground Execution:**\n- You should decide whether commands should run in background or foreground based on their nature:\n- Use background execution (is_background: true) for:\n  - Long-running development servers: `npm run start`, `npm run dev`, `yarn dev`, `bun run start`\n  - Build watchers: `npm run watch`, `webpack --watch`\n  - Database servers: `mongod`, `mysql`, `redis-server`\n  - Web servers: `python -m http.server`, `php -S localhost:8000`\n  - Any command expected to run indefinitely until manually stopped\n\n  - Command is executed as a subprocess that leads its own process group. Command process group can be terminated as `kill -- -PGID` or signaled as `kill -s SIGNAL -- -PGID`.\n- Use foreground execution (is_background: false) for:\n  - One-time commands: `ls`, `cat`, `grep`\n  - Build commands: `npm run build`, `make`\n  - Installation commands: `npm install`, `pip install`\n  - Git operations: `git commit`, `git push`\n  - Test runs: `npm test`, `pytest`\n".to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            [
                (
                    "command".to_string(),
                    JsonSchema::string(Some(
                        "Exact bash command to execute as `bash -c <command>`".to_string(),
                    )),
                ),
                (
                    "is_background".to_string(),
                    JsonSchema::boolean(Some("Optional: Whether to run the command in background. If not specified, defaults to false (foreground execution). Explicitly set to true for long-running processes like development servers, watchers, or daemons that should continue running without blocking further commands.".to_string())),
                ),
                (
                    "timeout".to_string(),
                    JsonSchema::number(Some("Optional timeout in milliseconds (max 600000)".to_string())),
                ),
                (
                    "description".to_string(),
                    JsonSchema::string(Some("Brief description of the command for the user. Be specific and concise. Ideally a single sentence. Can be up to 3 sentences for clarity. No line breaks.".to_string())),
                ),
                (
                    "directory".to_string(),
                    JsonSchema::string(Some("(OPTIONAL) The absolute path of the directory to run the command in. If not provided, the project root directory is used. Must be a directory within the workspace and must already exist.".to_string())),
                ),
            ],
            Some(vec!["command".to_string()]),
            None,
        ),
        output_schema: None,
    })
}

pub fn create_qwen_code_todo_write_tool() -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: "todo_write".to_string(),
        description: "Create or update the current todo list.".to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            [(
                "todos".to_string(),
                JsonSchema::array(
                    JsonSchema::object(
                        [
                            (
                                "id".to_string(),
                                JsonSchema::string(Some("Stable todo id.".to_string())),
                            ),
                            (
                                "content".to_string(),
                                JsonSchema::string(Some("Todo text.".to_string())),
                            ),
                            ("status".to_string(), {
                                JsonSchema::string_enum(
                                    vec![
                                        "pending".into(),
                                        "in_progress".into(),
                                        "completed".into(),
                                    ],
                                    Some("Todo status.".to_string()),
                                )
                            }),
                        ],
                        Some(vec![
                            "id".to_string(),
                            "content".to_string(),
                            "status".to_string(),
                        ]),
                        None,
                    ),
                    Some("Full todo list replacement.".to_string()),
                ),
            )],
            Some(vec!["todos".to_string()]),
            None,
        ),
        output_schema: None,
    })
}

pub fn create_qwen_code_ask_user_question_tool() -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: "ask_user_question".to_string(),
        description: "Ask the user structured questions with concrete answer options.".to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            [(
                "questions".to_string(),
                JsonSchema::array(
                    JsonSchema::object(
                        [
                            (
                                "question".to_string(),
                                JsonSchema::string(Some("Question text.".to_string())),
                            ),
                            (
                                "header".to_string(),
                                JsonSchema::string(Some("Short header label.".to_string())),
                            ),
                            (
                                "options".to_string(),
                                JsonSchema::array(
                                    JsonSchema::object(
                                        [
                                            (
                                                "label".to_string(),
                                                JsonSchema::string(Some(
                                                    "Short option label.".to_string(),
                                                )),
                                            ),
                                            (
                                                "description".to_string(),
                                                JsonSchema::string(Some(
                                                    "Option impact or tradeoff.".to_string(),
                                                )),
                                            ),
                                        ],
                                        Some(vec!["label".to_string(), "description".to_string()]),
                                        None,
                                    ),
                                    Some("Answer options.".to_string()),
                                ),
                            ),
                            ("multiSelect".to_string(), {
                                let mut schema = JsonSchema::boolean(Some(
                                    "Allow multiple answers.".to_string(),
                                ));
                                schema.default_value = Some(json!(false));
                                schema
                            }),
                        ],
                        Some(vec![
                            "question".to_string(),
                            "header".to_string(),
                            "options".to_string(),
                        ]),
                        None,
                    ),
                    Some("Questions to present to the user.".to_string()),
                ),
            )],
            Some(vec!["questions".to_string()]),
            None,
        ),
        output_schema: None,
    })
}

pub fn create_qwen_code_agent_tool(agent_type_description: String) -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: "agent".to_string(),
        description: "Delegate a bounded subtask to a subagent.".to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            [
                (
                    "description".to_string(),
                    JsonSchema::string(Some("Short task description.".to_string())),
                ),
                (
                    "prompt".to_string(),
                    JsonSchema::string(Some("Detailed instructions for the subagent.".to_string())),
                ),
                (
                    "subagent_type".to_string(),
                    JsonSchema::string(Some(agent_type_description)),
                ),
                ("run_in_background".to_string(), {
                    let mut schema = JsonSchema::boolean(Some(
                        "Run the subagent in the background.".to_string(),
                    ));
                    schema.default_value = Some(json!(false));
                    schema
                }),
            ],
            Some(vec!["description".to_string(), "prompt".to_string()]),
            None,
        ),
        output_schema: None,
    })
}

pub fn create_qwen_code_exit_plan_mode_tool() -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: "exit_plan_mode".to_string(),
        description: "Request user approval for the current plan.".to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            [(
                "options".to_string(),
                JsonSchema::array(
                    JsonSchema::object(
                        [
                            (
                                "label".to_string(),
                                JsonSchema::string(Some("Option label.".to_string())),
                            ),
                            (
                                "description".to_string(),
                                JsonSchema::string(Some("Option tradeoff.".to_string())),
                            ),
                        ],
                        Some(vec!["label".to_string(), "description".to_string()]),
                        None,
                    ),
                    Some("Optional plan choices.".to_string()),
                ),
            )],
            None,
            None,
        ),
        output_schema: None,
    })
}

pub fn create_qwen_code_monitor_tool() -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: "monitor".to_string(),
        description: "Inspect output from a background task.".to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            [
                (
                    "task_id".to_string(),
                    JsonSchema::string(Some("The background task ID to inspect.".to_string())),
                ),
                ("block".to_string(), {
                    let mut schema =
                        JsonSchema::boolean(Some("Wait for task completion.".to_string()));
                    schema.default_value = Some(json!(false));
                    schema
                }),
                ("timeout".to_string(), {
                    let mut schema = JsonSchema::integer(Some(
                        "Maximum number of seconds to wait when block=true.".to_string(),
                    ));
                    schema.default_value = Some(json!(30));
                    schema.minimum = Some(0);
                    schema.maximum = Some(3_600);
                    schema
                }),
            ],
            Some(vec!["task_id".to_string()]),
            None,
        ),
        output_schema: None,
    })
}

pub fn create_qwen_code_task_stop_tool() -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: "task_stop".to_string(),
        description: "Stop a running background task.".to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            [
                (
                    "task_id".to_string(),
                    JsonSchema::string(Some("The background task ID to stop.".to_string())),
                ),
                (
                    "reason".to_string(),
                    nullable_string("Optional reason recorded when the task is stopped."),
                ),
            ],
            Some(vec!["task_id".to_string()]),
            None,
        ),
        output_schema: None,
    })
}
