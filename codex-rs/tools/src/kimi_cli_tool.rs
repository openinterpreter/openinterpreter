use crate::JsonSchema;
use crate::ResponsesApiTool;
use crate::ToolSpec;
use serde_json::json;

const KIMI_CLI_ASK_USER_QUESTION_DESCRIPTION: &str = r#"Use this tool when you need to ask the user questions with structured options during execution. This allows you to:
1. Collect user preferences or requirements before proceeding
2. Resolve ambiguous or underspecified instructions
3. Let the user decide between implementation approaches as you work
4. Present concrete options when multiple valid directions exist

**When NOT to use:**
- When you can infer the answer from context — be decisive and proceed
- Trivial decisions that don't materially affect the outcome

Overusing this tool interrupts the user's flow. Only use it when the user's input genuinely changes your next action.

**Usage notes:**
- Users always have an "Other" option for custom input — don't create one yourself
- Use multi_select to allow multiple answers to be selected for a question
- Keep option labels concise (1-5 words), use descriptions for trade-offs and details
- Each question should have 2-4 meaningful, distinct options
- You can ask 1-4 questions at a time; group related questions to minimize interruptions
- If you recommend a specific option, list it first and append "(Recommended)" to its label
"#;

const KIMI_CLI_SET_TODO_LIST_DESCRIPTION: &str = r#"Manage your todo list for tracking task progress.

Todo list is a simple yet powerful tool to help you get things done. You typically want to use this tool when the given task involves multiple subtasks/milestones, or, multiple tasks are given in a single request. This tool can help you to break down the task and track the progress.

**Usage modes:**

- **Update mode**: Pass `todos` to set the entire todo list. The previous list is replaced.
- **Query mode**: Omit `todos` (or pass null) to retrieve the current todo list without changes.
- **Clear mode**: Pass an empty array `[]` to clear all todos.

This is the only todo list tool available to you. That said, each time you want to update the todo list, you need to provide the whole list. Make sure to maintain the todo items and their statuses properly.

Once you finished a subtask/milestone, remember to update the todo list to reflect the progress. Also, you can give yourself a self-encouragement to keep you motivated.

Abusing this tool to track too small steps will just waste your time and make your context messy. For example, here are some cases you should not use this tool:

- When the user just simply ask you a question. E.g. "What language and framework is used in the project?", "What is the best practice for x?"
- When it only takes a few steps/tool calls to complete the task. E.g. "Fix the unit test function 'test_xxx'", "Refactor the function 'xxx' to make it more solid."
- When the user prompt is very specific and the only thing you need to do is brainlessly following the instructions. E.g. "Replace xxx to yyy in the file zzz", "Create a file xxx with content yyy."

However, do not get stuck in a rut. Be flexible. Sometimes, you may try to use todo list at first, then realize the task is too simple and you can simply stop using it; or, sometimes, you may realize the task is complex after a few steps and then you can start using todo list to break it down.

IMPORTANT: Do not call this tool repeatedly without making real progress on at least one task between calls. If you are unsure about the current state, use Query mode (omit `todos`) to check before updating. If you find yourself unable to advance any task with your available tools, inform the user about what is blocking you instead of replanning. Repeatedly updating the todo list without doing actual work is counterproductive.
"#;

const KIMI_CLI_SHELL_DESCRIPTION: &str = r#"Execute a bash (`/bin/bash`) command. Use this tool to explore the filesystem, edit files, run scripts, get system information, etc.

**Output:**
The stdout and stderr will be combined and returned as a string. The output may be truncated if it is too long. If the command failed, the exit code will be provided in a system tag.

If `run_in_background=true`, the command will be started as a background task and this tool will return a task ID instead of waiting for command completion. When doing that, you must provide a short `description`. You will be automatically notified when the task completes. Use `TaskOutput` for a non-blocking status/output snapshot, and only set `block=true` when you explicitly want to wait for completion. Use `TaskStop` only if the task must be cancelled. For human users in the interactive shell, background tasks are managed through `/task` only; do not suggest `/task list`, `/task output`, `/task stop`, `/tasks`, or any other invented shell subcommands.

**Guidelines for safety and security:**
- Each shell tool call will be executed in a fresh shell environment. The shell variables, current working directory changes, and the shell history is not preserved between calls.
- The tool call will return after the command is finished. You shall not use this tool to execute an interactive command or a command that may run forever. For possibly long-running commands, you shall set `timeout` argument to a reasonable value.
- Avoid using `..` to access files or directories outside of the working directory.
- Avoid modifying files outside of the working directory unless explicitly instructed to do so.
- Never run commands that require superuser privileges unless explicitly instructed to do so.

**Guidelines for efficiency:**
- For multiple related commands, use `&&` to chain them in a single call, e.g. `cd /path && ls -la`
- Use `;` to run commands sequentially regardless of success/failure
- Use `||` for conditional execution (run second command only if first fails)
- Use pipe operations (`|`) and redirections (`>`, `>>`) to chain input and output between commands
- Always quote file paths containing spaces with double quotes (e.g., cd "/path with spaces/")
- Use `if`, `case`, `for`, `while` control flows to execute complex logic in a single call.
- Verify directory structure before create/edit/delete files or directories to reduce the risk of failure.
- Prefer `run_in_background=true` for long-running builds, tests, watchers, or servers when you need the conversation to continue before the command finishes.
- After starting a background task, do not guess its outcome. Rely on the automatic completion notification whenever possible. Use `TaskOutput` for non-blocking progress snapshots by default, and set `block=true` only when you intentionally want to wait.
- If you need to tell a human shell user how to manage background tasks, only mention `/task`. Do not invent `/task list`, `/task output`, `/task stop`, or `/tasks`.

**Commands available:**
- Shell environment: cd, pwd, export, unset, env
- File system operations: ls, find, mkdir, rm, cp, mv, touch, chmod, chown
- File viewing/editing: cat, grep, head, tail, diff, patch
- Text processing: awk, sed, sort, uniq, wc
- System information/operations: ps, kill, top, df, free, uname, whoami, id, date
- Network operations: curl, wget, ping, telnet, ssh
- Archive operations: tar, zip, unzip
- Other: Other commands available in the shell environment. Check the existence of a command by running `which <command>` before using it.
"#;

const KIMI_CLI_TASK_LIST_DESCRIPTION: &str = r#"List background tasks from the current session.

Use this when you need to re-enumerate which background tasks still exist, especially after context compaction or when you are no longer confident which task IDs are still active.

Guidelines:

- Prefer the default `active_only=true` unless you specifically need completed or failed tasks.
- Use `TaskOutput` to inspect one task in detail after you have identified the correct task ID.
- Do not guess which tasks are still running when you can call this tool directly.
- This tool is read-only and safe to use in plan mode.
"#;

const KIMI_CLI_TASK_OUTPUT_DESCRIPTION: &str = r#"Retrieve output from a running or completed background task.

Use this after `Shell(run_in_background=true)` when you need to inspect progress or explicitly wait for completion.

Guidelines:
- Prefer relying on automatic completion notifications. Use this tool only when you need task output before the automatic notification arrives.
- By default this tool is non-blocking and returns a current status/output snapshot.
- Use `block=true` only when you intentionally want to wait for completion or timeout.
- This tool returns structured task metadata, a fixed-size output preview, and an `output_path` for the full log.
- When the preview is truncated, use `ReadFile` with the returned `output_path` to inspect the full log in pages.
- This tool works with the generic background task system and should remain the primary read path for future task types, not just bash.
"#;

const KIMI_CLI_TASK_STOP_DESCRIPTION: &str = r#"Stop a running background task.

Use this only when a background task must be cancelled. For normal task completion, prefer waiting for the automatic notification or using `TaskOutput`.

Guidelines:
- This is a generic task stop capability, not a bash-specific kill tool.
- Use it sparingly because stopping a task is destructive and may leave partial side effects.
- If the task is already complete, this tool will simply return its current state.
"#;

const KIMI_CLI_READ_FILE_DESCRIPTION: &str = r#"Read text content from a file.

**Tips:**
- Make sure you follow the description of each tool parameter.
- A `<system>` tag will be given before the read file content.
- The system will notify you when there is anything wrong when reading the file.
- This tool is a tool that you typically want to use in parallel. Always read multiple files in one response when possible.
- This tool can only read text files. To read images or videos, use other appropriate tools. To list directories, use the Glob tool or `ls` command via the Shell tool. To read other file types, use appropriate commands via the Shell tool.
- If the file doesn't exist or path is invalid, an error will be returned.
- If you want to search for a certain content/pattern, prefer Grep tool over ReadFile.
- Content will be returned with a line number before each line like `cat -n` format.
- Use `line_offset` and `n_lines` parameters when you only need to read a part of the file.
- Use negative `line_offset` to read from the end of the file (e.g. `line_offset=-100` reads the last 100 lines). This is useful for viewing the tail of log files. The absolute value cannot exceed 1000.
- The tool always returns the total number of lines in the file in its message, which you can use to plan subsequent reads.
- The maximum number of lines that can be read at once is 1000.
- Any lines longer than 2000 characters will be truncated, ending with "...".
"#;

const KIMI_CLI_READ_MEDIA_FILE_DESCRIPTION: &str = r#"Read media content from a file.

**Tips:**
- Make sure you follow the description of each tool parameter.
- A `<system>` tag will be given before the read file content.
- The system will notify you when there is anything wrong when reading the file.
- This tool is a tool that you typically want to use in parallel. Always read multiple files in one response when possible.
- This tool can only read image or video files. To read other types of files, use the ReadFile tool. To list directories, use the Glob tool or `ls` command via the Shell tool.
- If the file doesn't exist or path is invalid, an error will be returned.
- The maximum size that can be read is 100MB. An error will be returned if the file is larger than this limit.
- The media content will be returned in a form that you can directly view and understand.

**Capabilities**
- This tool supports image and video files for the current model.
"#;

const KIMI_CLI_GLOB_DESCRIPTION: &str = r#"Find files and directories using glob patterns. This tool supports standard glob syntax like `*`, `?`, and `**` for recursive searches.

**When to use:**
- Find files matching specific patterns (e.g., all Python files: `*.py`)
- Search for files recursively in subdirectories (e.g., `src/**/*.js`)
- Locate configuration files (e.g., `*.config.*`, `*.json`)
- Find test files (e.g., `test_*.py`, `*_test.go`)

**Example patterns:**
- `*.py` - All Python files in current directory
- `src/**/*.js` - All JavaScript files in src directory recursively
- `test_*.py` - Python test files starting with "test_"
- `*.config.{js,ts}` - Config files with .js or .ts extension

**Bad example patterns:**
- `**`, `**/*.py` - Any pattern starting with '**' will be rejected. Because it would recursively search all directories and subdirectories, which is very likely to yield large result that exceeds your context size. Always use more specific patterns like `src/**/*.py` instead.
- `node_modules/**/*.js` - Although this does not start with '**', it would still highly possible to yield large result because `node_modules` is well-known to contain too many directories and files. Avoid recursively searching in such directories, other examples include `venv`, `.venv`, `__pycache__`, `target`. If you really need to search in a dependency, use more specific patterns like `node_modules/react/src/*` instead.
"#;

const KIMI_CLI_GREP_DESCRIPTION: &str = r#"A powerful search tool based-on ripgrep.

**Tips:**
- ALWAYS use Grep tool instead of running `grep` or `rg` command with Shell tool.
- Use the ripgrep pattern syntax, not grep syntax. E.g. you need to escape braces like `\\{` to search for `{`.
- Hidden files (dotfiles like `.gitlab-ci.yml`, `.eslintrc.json`) are always searched. To also search files excluded by `.gitignore` (e.g. `node_modules`, build outputs), set `include_ignored` to `true`. Sensitive files (such as `.env`) are still skipped for safety, even when `include_ignored` is `true`.
"#;

const KIMI_CLI_WRITE_FILE_DESCRIPTION: &str = r#"Write content to a file.

**Tips:**
- When `mode` is not specified, it defaults to `overwrite`. Always write with caution.
- When the content to write is too long (e.g. > 100 lines), use this tool multiple times instead of a single call. Use `overwrite` mode at the first time, then use `append` mode after the first write.
"#;

const KIMI_CLI_STR_REPLACE_FILE_DESCRIPTION: &str = r#"Replace specific strings within a specified file.

**Tips:**
- Only use this tool on text files.
- Multi-line strings are supported.
- Can specify a single edit or a list of edits in one call.
- You should prefer this tool over WriteFile tool and Shell `sed` command.
"#;

const KIMI_CLI_EXIT_PLAN_MODE_DESCRIPTION: &str = r#"Use this tool when you are in plan mode and have finished writing your plan to the plan file and are ready for user approval.

## How This Tool Works
- You should have already written your plan to the plan file specified in the plan mode reminder.
- This tool does NOT take the plan content as a parameter — it reads the plan from the file you wrote.
- The user will see the contents of your plan file when they review it.

## When to Use
Only use this tool for tasks that require planning implementation steps. For research tasks (searching files, reading code, understanding the codebase), do NOT use this tool.

## Multiple Approaches
If your plan contains multiple alternative approaches:
- Pass them via the `options` parameter so the user can choose which approach to execute.
- Each option should have a concise label and a brief description of trade-offs.
- If you recommend one option, append "(Recommended)" to its label.
- The user will see all options alongside Reject and Revise choices.
- Provide 2-3 options at most (the system appends a "Reject" option automatically, so the total shown to the user is 3-4).
- Do NOT use "Reject", "Revise", or "Approve" as option labels — these are reserved by the system.

## Before Using
- If you have unresolved questions, use AskUserQuestion first.
- If you have multiple approaches and haven't narrowed down yet, consider using AskUserQuestion first to let the user choose, then write a plan for the chosen approach only.
- Once your plan is finalized, use THIS tool to request approval.
- Do NOT use AskUserQuestion to ask "Is this plan OK?" or "Should I proceed?" — that is exactly what ExitPlanMode does.
- If rejected, revise based on feedback and call ExitPlanMode again.
"#;

const KIMI_CLI_ENTER_PLAN_MODE_DESCRIPTION: &str = r#"Use this tool proactively when you're about to start a non-trivial implementation task.
Getting user sign-off on your approach before writing code prevents wasted effort.

Use it when ANY of these conditions apply:

1. New Feature Implementation — e.g. "Add a caching layer to the API"
2. Multiple Valid Approaches — e.g. "Optimize database queries" (indexing vs rewrite vs caching)
3. Code Modifications — e.g. "Refactor auth module to support OAuth"
4. Architectural Decisions — e.g. "Add WebSocket support"
5. Multi-File Changes — involves more than 2-3 files
6. Unclear Requirements — need exploration to understand scope
7. User Preferences Matter — if you'd use AskUserQuestion to clarify approach, use EnterPlanMode instead

Yolo mode note:
- Yolo mode users chose continuous execution.
- In yolo mode, use EnterPlanMode only when the user explicitly asks for planning or when
  there is exceptional architectural ambiguity that requires user input before proceeding.

When NOT to use:
- Single-line or few-line fixes (typos, obvious bugs, small tweaks)
- User gave very specific, detailed instructions
- Pure research/exploration tasks

## What Happens in Plan Mode
In plan mode, you will:
1. Identify 2-3 key questions about the codebase that are critical to your plan. If you are not confident about the codebase structure or relevant code paths, use `Agent(subagent_type="explore")` to investigate these questions first — this is strongly recommended for non-trivial tasks.
2. Explore the codebase using Glob, Grep, ReadFile (read-only) for any remaining quick lookups
3. Design an implementation approach based on your findings
4. Write your plan to a plan file
5. Present your plan to the user via ExitPlanMode for approval
"#;

pub fn create_kimi_cli_ask_user_question_tool() -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: "AskUserQuestion".to_string(),
        description: KIMI_CLI_ASK_USER_QUESTION_DESCRIPTION.to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            [("questions".to_string(), {
                let mut schema = JsonSchema::array(
                    kimi_question_schema(),
                    Some("The questions to ask the user (1-4 questions).".to_string()),
                );
                schema.min_items = Some(1);
                schema.max_items = Some(4);
                schema
            })],
            Some(vec!["questions".to_string()]),
            None,
        ),
        output_schema: None,
    })
}

pub fn create_kimi_cli_set_todo_list_tool() -> ToolSpec {
    let mut todos_schema = JsonSchema::any_of(
        vec![
            JsonSchema::array(kimi_todo_item_schema(), None),
            JsonSchema::null(None),
        ],
        Some(
            "The updated todo list. If not provided, returns the current todo list without making changes."
                .to_string(),
        ),
    );
    todos_schema.default_value = Some(json!(null));

    ToolSpec::Function(ResponsesApiTool {
        name: "SetTodoList".to_string(),
        description: KIMI_CLI_SET_TODO_LIST_DESCRIPTION.to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object([("todos".to_string(), todos_schema)], None, None),
        output_schema: None,
    })
}

pub fn create_kimi_cli_shell_tool() -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: "Shell".to_string(),
        description: KIMI_CLI_SHELL_DESCRIPTION.to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            [
                (
                    "command".to_string(),
                    JsonSchema::string(Some("The command to execute.".to_string())),
                ),
                ("timeout".to_string(), {
                    let mut schema = JsonSchema::integer(Some(
                            "The timeout in seconds for the command to execute. If the command takes longer than this, it will be killed."
                                .to_string(),
                        ));
                    schema.default_value = Some(json!(60));
                    schema.minimum = Some(1);
                    schema.maximum = Some(86_400);
                    schema
                }),
                ("run_in_background".to_string(), {
                    let mut schema = JsonSchema::boolean(Some(
                        "Whether to run the command as a background task.".to_string(),
                    ));
                    schema.default_value = Some(json!(false));
                    schema
                }),
                ("description".to_string(), {
                    let mut schema = JsonSchema::string(Some(
                            "A short description for the background task. Required when run_in_background=true."
                                .to_string(),
                        ));
                    schema.default_value = Some(json!(""));
                    schema
                }),
            ],
            Some(vec!["command".to_string()]),
            None,
        ),
        output_schema: None,
    })
}

pub fn create_kimi_cli_read_file_tool() -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: "ReadFile".to_string(),
        description: KIMI_CLI_READ_FILE_DESCRIPTION.to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            [
                (
                    "path".to_string(),
                    JsonSchema::string(Some(
                        "The path to the file to read. Absolute paths are required when reading files outside the working directory."
                            .to_string(),
                    )),
                ),
                (
                    "line_offset".to_string(),
                    {
                        let mut schema = JsonSchema::integer(Some(
                            "The line number to start reading from. By default read from the beginning of the file. Set this when the file is too large to read at once. Negative values read from the end of the file (e.g. -100 reads the last 100 lines). The absolute value of negative offset cannot exceed 1000."
                                .to_string(),
                        ));
                        schema.default_value = Some(json!(1));
                        schema
                    },
                ),
                (
                    "n_lines".to_string(),
                    {
                        let mut schema = JsonSchema::integer(Some(
                            "The number of lines to read. By default read up to 1000 lines, which is the max allowed value. Set this value when the file is too large to read at once.".to_string(),
                        ));
                        schema.default_value = Some(json!(1000));
                        schema.minimum = Some(1);
                        schema
                    },
                ),
            ],
            Some(vec!["path".to_string()]),
            None,
        ),
        output_schema: None,
    })
}

pub fn create_kimi_cli_glob_tool() -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: "Glob".to_string(),
        description: KIMI_CLI_GLOB_DESCRIPTION.to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            [
                (
                    "pattern".to_string(),
                    JsonSchema::string(Some(
                        "Glob pattern to match files/directories.".to_string(),
                    )),
                ),
                ("directory".to_string(), {
                    let mut schema = JsonSchema::any_of(
                        vec![JsonSchema::string(None), JsonSchema::null(None)],
                        Some(
                            "Absolute path to the directory to search in (defaults to working directory)."
                                .to_string(),
                        ),
                        );
                    schema.default_value = Some(json!(null));
                    schema
                }),
                ("include_dirs".to_string(), {
                    let mut schema = JsonSchema::boolean(Some(
                        "Whether to include directories in results.".to_string(),
                    ));
                    schema.default_value = Some(json!(true));
                    schema
                }),
            ],
            Some(vec!["pattern".to_string()]),
            None,
        ),
        output_schema: None,
    })
}

pub fn create_kimi_cli_grep_tool() -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: "Grep".to_string(),
        description: KIMI_CLI_GREP_DESCRIPTION.to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            [
                (
                    "pattern".to_string(),
                    JsonSchema::string(Some(
                        "The regular expression pattern to search for in file contents".to_string(),
                    )),
                ),
                ("path".to_string(), {
                    let mut schema = JsonSchema::string(Some(
                            "File or directory to search in. Defaults to current working directory. If specified, it must be an absolute path."
                                .to_string(),
                        ));
                    schema.default_value = Some(json!("."));
                    schema
                }),
                ("glob".to_string(), {
                    let mut schema = JsonSchema::any_of(
                        vec![JsonSchema::string(None), JsonSchema::null(None)],
                        Some(
                            "Glob pattern to filter files (e.g. `*.js`, `*.{ts,tsx}`). No filter by default."
                                .to_string(),
                        ),
                        );
                    schema.default_value = Some(json!(null));
                    schema
                }),
                ("output_mode".to_string(), {
                    let mut schema = JsonSchema::string(
                            Some(
                                "`content`: Show matching lines (supports `-B`, `-A`, `-C`, `-n`, `head_limit`); `files_with_matches`: Show file paths (supports `head_limit`); `count_matches`: Show total number of matches. Defaults to `files_with_matches`."
                                    .to_string(),
                            ),
                        );
                    schema.default_value = Some(json!("files_with_matches"));
                    schema
                }),
                ("-B".to_string(), {
                    let mut schema = JsonSchema::any_of(
                            vec![JsonSchema::integer(None), JsonSchema::null(None)],
                            Some(
                                "Number of lines to show before each match (the `-B` option). Requires `output_mode` to be `content`."
                                    .to_string(),
                            ),
                        );
                    schema.default_value = Some(json!(null));
                    schema
                }),
                ("-A".to_string(), {
                    let mut schema = JsonSchema::any_of(
                            vec![JsonSchema::integer(None), JsonSchema::null(None)],
                            Some(
                                "Number of lines to show after each match (the `-A` option). Requires `output_mode` to be `content`."
                                    .to_string(),
                            ),
                        );
                    schema.default_value = Some(json!(null));
                    schema
                }),
                ("-C".to_string(), {
                    let mut schema = JsonSchema::any_of(
                            vec![JsonSchema::integer(None), JsonSchema::null(None)],
                            Some(
                                "Number of lines to show before and after each match (the `-C` option). Requires `output_mode` to be `content`."
                                    .to_string(),
                            ),
                        );
                    schema.default_value = Some(json!(null));
                    schema
                }),
                ("-n".to_string(), {
                    let mut schema = JsonSchema::boolean(Some(
                            "Show line numbers in output (the `-n` option). Requires `output_mode` to be `content`. Defaults to true."
                                .to_string(),
                        ));
                    schema.default_value = Some(json!(true));
                    schema
                }),
                ("-i".to_string(), {
                    let mut schema = JsonSchema::boolean(Some(
                        "Case insensitive search (the `-i` option).".to_string(),
                    ));
                    schema.default_value = Some(json!(false));
                    schema
                }),
                ("type".to_string(), {
                    let mut schema = JsonSchema::any_of(
                        vec![JsonSchema::string(None), JsonSchema::null(None)],
                        Some(
                            "File type to search. Examples: py, rust, js, ts, go, java, etc. More efficient than `glob` for standard file types."
                                .to_string(),
                        ),
                        );
                    schema.default_value = Some(json!(null));
                    schema
                }),
                ("head_limit".to_string(), {
                    let mut integer_schema = JsonSchema::integer(None);
                    integer_schema.minimum = Some(0);
                    let mut schema = JsonSchema::any_of(
                            vec![integer_schema, JsonSchema::null(None)],
                            Some(
                                "Limit output to first N lines/entries, equivalent to `| head -N`. Works across all output modes: content (limits output lines), files_with_matches (limits file paths), count_matches (limits count entries). Defaults to 250. Pass 0 for unlimited (use sparingly — large result sets waste context)."
                                    .to_string(),
                            ),
                        );
                    schema.default_value = Some(json!(250));
                    schema
                }),
                ("offset".to_string(), {
                    let mut schema = JsonSchema::integer(Some(
                            "Skip first N lines/entries before applying head_limit, equivalent to `| tail -n +N | head -N`. Works across all output modes. Defaults to 0."
                                .to_string(),
                        ));
                    schema.default_value = Some(json!(0));
                    schema.minimum = Some(0);
                    schema
                }),
                ("multiline".to_string(), {
                    let mut schema = JsonSchema::boolean(Some(
                            "Enable multiline mode where `.` matches newlines and patterns can span lines (the `-U` and `--multiline-dotall` options). By default, multiline mode is disabled."
                                .to_string(),
                        ));
                    schema.default_value = Some(json!(false));
                    schema
                }),
                ("include_ignored".to_string(), {
                    let mut schema = JsonSchema::boolean(Some(
                            "Include files that are ignored by `.gitignore`, `.ignore`, and other ignore rules. Useful for searching gitignored artifacts such as build outputs (e.g. `dist/`, `build/`) or `node_modules`. Sensitive files (like `.env`) remain filtered by the sensitive-file protection layer. Defaults to false."
                                .to_string(),
                        ));
                    schema.default_value = Some(json!(false));
                    schema
                }),
            ],
            Some(vec!["pattern".to_string()]),
            None,
        ),
        output_schema: None,
    })
}

pub fn create_kimi_cli_write_file_tool() -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: "WriteFile".to_string(),
        description: KIMI_CLI_WRITE_FILE_DESCRIPTION.to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            [
                (
                    "path".to_string(),
                    JsonSchema::string(Some(
                        "The path to the file to write. Absolute paths are required when writing files outside the working directory."
                            .to_string(),
                    )),
                ),
                (
                    "content".to_string(),
                    JsonSchema::string(Some("The content to write to the file".to_string())),
                ),
                (
                    "mode".to_string(),
                    {
                        let mut schema = JsonSchema::string_enum(
                            vec!["overwrite".into(), "append".into()],
                            Some(
                                "The mode to use to write to the file. Two modes are supported: `overwrite` for overwriting the whole file and `append` for appending to the end of an existing file."
                                    .to_string(),
                            ),
                        );
                        schema.default_value = Some(json!("overwrite"));
                        schema
                    },
                ),
            ],
            Some(vec!["path".to_string(), "content".to_string()]),
            None,
        ),
        output_schema: None,
    })
}

pub fn create_kimi_cli_str_replace_file_tool() -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: "StrReplaceFile".to_string(),
        description: KIMI_CLI_STR_REPLACE_FILE_DESCRIPTION.to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            [
                (
                    "path".to_string(),
                    JsonSchema::string(Some(
                        "The path to the file to edit. Absolute paths are required when editing files outside the working directory."
                            .to_string(),
                    )),
                ),
                (
                    "edit".to_string(),
                    JsonSchema::any_of(
                        vec![
                            kimi_edit_schema(),
                            JsonSchema::array(kimi_edit_schema(), None),
                        ],
                        Some(
                            "The edit(s) to apply to the file. You can provide a single edit or a list of edits here."
                                .to_string(),
                        ),
                    ),
                ),
            ],
            Some(vec!["path".to_string(), "edit".to_string()]),
            None,
        ),
        output_schema: None,
    })
}

pub fn create_kimi_cli_search_web_tool() -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: "SearchWeb".to_string(),
        description:
            "WebSearch tool allows you to search on the internet to get latest information, including news, documents, release notes, blog posts, papers, etc.\n"
                .to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            [
                (
                    "query".to_string(),
                    JsonSchema::string(Some("The query text to search for.".to_string())),
                ),
                (
                    "limit".to_string(),
                    {
                        let mut schema = JsonSchema::integer(Some(
                            "The number of results to return. Typically you do not need to set this value. When the results do not contain what you need, you probably want to give a more concrete query."
                                .to_string(),
                        ));
                        schema.default_value = Some(json!(5));
                        schema.minimum = Some(1);
                        schema.maximum = Some(20);
                        schema
                    },
                ),
                (
                    "include_content".to_string(),
                    {
                        let mut schema = JsonSchema::boolean(Some(
                            "Whether to include the content of the web pages in the results. It can consume a large amount of tokens when this is set to True. You should avoid enabling this when `limit` is set to a large value."
                                .to_string(),
                        ));
                        schema.default_value = Some(json!(false));
                        schema
                    },
                ),
            ],
            Some(vec!["query".to_string()]),
            None,
        ),
        output_schema: None,
    })
}

pub fn create_kimi_cli_fetch_url_tool() -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: "FetchURL".to_string(),
        description: "Fetch a web page from a URL and extract main text content from it.\n"
            .to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            [(
                "url".to_string(),
                JsonSchema::string(Some("The URL to fetch content from.".to_string())),
            )],
            Some(vec!["url".to_string()]),
            None,
        ),
        output_schema: None,
    })
}

pub fn create_kimi_cli_task_list_tool() -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: "TaskList".to_string(),
        description: KIMI_CLI_TASK_LIST_DESCRIPTION.to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            [
                ("active_only".to_string(), {
                    let mut schema = JsonSchema::boolean(Some(
                        "Whether to list only non-terminal background tasks.".to_string(),
                    ));
                    schema.default_value = Some(json!(true));
                    schema
                }),
                ("limit".to_string(), {
                    let mut schema =
                        JsonSchema::integer(Some("Maximum number of tasks to return.".to_string()));
                    schema.default_value = Some(json!(20));
                    schema.minimum = Some(1);
                    schema.maximum = Some(100);
                    schema
                }),
            ],
            None,
            None,
        ),
        output_schema: None,
    })
}

pub fn create_kimi_cli_task_output_tool() -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: "TaskOutput".to_string(),
        description: KIMI_CLI_TASK_OUTPUT_DESCRIPTION.to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            [
                (
                    "task_id".to_string(),
                    JsonSchema::string(Some("The background task ID to inspect.".to_string())),
                ),
                ("block".to_string(), {
                    let mut schema = JsonSchema::boolean(Some(
                        "Whether to wait for the task to finish before returning.".to_string(),
                    ));
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

pub fn create_kimi_cli_task_stop_tool() -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: "TaskStop".to_string(),
        description: KIMI_CLI_TASK_STOP_DESCRIPTION.to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            [
                (
                    "task_id".to_string(),
                    JsonSchema::string(Some("The background task ID to stop.".to_string())),
                ),
                ("reason".to_string(), {
                    let mut schema = JsonSchema::string(Some(
                        "Short reason recorded when the task is stopped.".to_string(),
                    ));
                    schema.default_value = Some(json!("Stopped by TaskStop"));
                    schema
                }),
            ],
            Some(vec!["task_id".to_string()]),
            None,
        ),
        output_schema: None,
    })
}

pub fn create_kimi_cli_read_media_file_tool() -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: "ReadMediaFile".to_string(),
        description: KIMI_CLI_READ_MEDIA_FILE_DESCRIPTION.to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            [(
                "path".to_string(),
                JsonSchema::string(Some(
                    "The path to the file to read. Absolute paths are required when reading files outside the working directory."
                        .to_string(),
                )),
            )],
            Some(vec!["path".to_string()]),
            None,
        ),
        output_schema: None,
    })
}

pub fn create_kimi_cli_exit_plan_mode_tool() -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: "ExitPlanMode".to_string(),
        description: KIMI_CLI_EXIT_PLAN_MODE_DESCRIPTION.to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            [("options".to_string(), {
                let mut options_array = JsonSchema::array(
                    {
                        let mut option = JsonSchema::object(
                            [
                                (
                                    "label".to_string(),
                                    JsonSchema::string(Some(
                                        "Short name for this option (1-8 words). Append '(Recommended)' if you recommend this option."
                                            .to_string(),
                                    )),
                                ),
                                (
                                    "description".to_string(),
                                    {
                                        let mut schema = JsonSchema::string(Some(
                                            "Brief summary of this approach and its trade-offs."
                                                .to_string(),
                                        ));
                                        schema.default_value = Some(json!(""));
                                        schema
                                    },
                                ),
                            ],
                            Some(vec!["label".to_string()]),
                            None,
                            );
                        option.description =
                            Some("A selectable approach/option within the plan.".to_string());
                        option
                    },
                    None,
                );
                options_array.max_items = Some(3);
                let mut schema = JsonSchema::any_of(
                        vec![options_array, JsonSchema::null(None)],
                        Some(
                            "When the plan contains multiple alternative approaches, list them here so the user can choose which one to execute. 2-3 options. Each option represents a distinct approach from the plan. Do not use 'Reject', 'Revise', 'Approve', or 'Reject and Exit' as labels."
                                .to_string(),
                        ),
                    );
                schema.default_value = Some(json!(null));
                schema
            })],
            None,
            None,
        ),
        output_schema: None,
    })
}

pub fn create_kimi_cli_enter_plan_mode_tool() -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: "EnterPlanMode".to_string(),
        description: KIMI_CLI_ENTER_PLAN_MODE_DESCRIPTION.to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object([], None, None),
        output_schema: None,
    })
}

fn kimi_todo_item_schema() -> JsonSchema {
    JsonSchema::object(
        [
            ("title".to_string(), {
                let mut schema = JsonSchema::string(Some("The title of the todo".to_string()));
                schema.min_length = Some(1);
                schema
            }),
            (
                "status".to_string(),
                JsonSchema::string_enum(
                    vec!["pending".into(), "in_progress".into(), "done".into()],
                    Some("The status of the todo".to_string()),
                ),
            ),
        ],
        Some(vec!["title".to_string(), "status".to_string()]),
        None,
    )
}

fn kimi_question_schema() -> JsonSchema {
    JsonSchema::object(
        [
            (
                "question".to_string(),
                JsonSchema::string(Some(
                    "A specific, actionable question. End with '?'.".to_string(),
                )),
            ),
            ("header".to_string(), {
                let mut schema = JsonSchema::string(Some(
                    "Short category tag (max 12 chars, e.g. 'Auth', 'Style').".to_string(),
                ));
                schema.default_value = Some(json!(""));
                schema
            }),
            ("options".to_string(), {
                let mut schema = JsonSchema::array(
                    JsonSchema::object(
                        [
                            (
                                "label".to_string(),
                                JsonSchema::string(Some(
                                    "Concise display text (1-5 words). If recommended, append '(Recommended)'."
                                        .to_string(),
                                )),
                            ),
                            ("description".to_string(), {
                                let mut schema = JsonSchema::string(Some(
                                    "Brief explanation of trade-offs or implications of choosing this option."
                                        .to_string(),
                                ));
                                schema.default_value = Some(json!(""));
                                schema
                            }),
                        ],
                        Some(vec!["label".to_string()]),
                        None,
                    ),
                    Some(
                        "2-4 meaningful, distinct options. Do NOT include an 'Other' option — the system adds one automatically."
                            .to_string(),
                    ),
                );
                schema.min_items = Some(2);
                schema.max_items = Some(4);
                schema
            }),
            ("multi_select".to_string(), {
                let mut schema = JsonSchema::boolean(Some(
                    "Whether the user can select multiple options.".to_string(),
                ));
                schema.default_value = Some(json!(false));
                schema
            }),
        ],
        Some(vec!["question".to_string(), "options".to_string()]),
        None,
    )
}

fn kimi_edit_schema() -> JsonSchema {
    JsonSchema::object(
        [
            (
                "old".to_string(),
                JsonSchema::string(Some(
                    "The old string to replace. Can be multi-line.".to_string(),
                )),
            ),
            (
                "new".to_string(),
                JsonSchema::string(Some(
                    "The new string to replace with. Can be multi-line.".to_string(),
                )),
            ),
            ("replace_all".to_string(), {
                let mut schema =
                    JsonSchema::boolean(Some("Whether to replace all occurrences.".to_string()));
                schema.default_value = Some(json!(false));
                schema
            }),
        ],
        Some(vec!["old".to_string(), "new".to_string()]),
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::create_kimi_cli_read_file_tool;
    use super::create_kimi_cli_shell_tool;
    use super::create_kimi_cli_str_replace_file_tool;
    use super::create_kimi_cli_write_file_tool;
    use crate::JsonSchema;
    use crate::ToolSpec;

    fn assert_property_order(schema: &JsonSchema, expected: &[&str]) {
        let serialized = serde_json::to_string(schema).expect("serialize schema");
        let mut last_position = 0;
        for property in expected {
            let needle = format!("\"{property}\":");
            let relative_position = serialized[last_position..]
                .find(&needle)
                .unwrap_or_else(|| panic!("missing property {property} in {serialized}"));
            last_position += relative_position + needle.len();
        }
    }

    fn parameters(tool: ToolSpec) -> JsonSchema {
        let ToolSpec::Function(tool) = tool else {
            panic!("expected function tool");
        };
        tool.parameters
    }

    #[test]
    fn kimi_shell_parameter_order_matches_kimi_code() {
        assert_property_order(
            &parameters(create_kimi_cli_shell_tool()),
            &["command", "timeout", "run_in_background", "description"],
        );
    }

    #[test]
    fn kimi_file_tool_parameter_order_matches_kimi_code() {
        assert_property_order(
            &parameters(create_kimi_cli_read_file_tool()),
            &["path", "line_offset", "n_lines"],
        );
        assert_property_order(
            &parameters(create_kimi_cli_write_file_tool()),
            &["path", "content", "mode"],
        );
        assert_property_order(
            &parameters(create_kimi_cli_str_replace_file_tool()),
            &["path", "edit"],
        );
    }
}
