use crate::client_common::Prompt;
use crate::harness::pi;
use codex_chat_wire_compat::ToolKinds;
use codex_chat_wire_compat::ToolOutputKind;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::openai_models::ModelInfo;
use serde_json::Value;
use serde_json::json;
use std::path::Path;

const LITTLE_CODER_SYSTEM_PROMPT: &str = include_str!("little_coder_system_prompt.md");

pub(crate) fn build_request(
    prompt: &Prompt,
    model_info: &ModelInfo,
) -> Result<(Value, ToolKinds), serde_json::Error> {
    let mut messages = vec![json!({
        "role": "system",
        "content": build_system_prompt(prompt),
    })];
    messages.extend(pi::build_messages(prompt.get_formatted_input())?);
    let tools = build_tools();
    let tool_kinds = tools
        .iter()
        .filter_map(|tool| {
            tool.get("function")
                .and_then(|function| function.get("name"))
                .and_then(Value::as_str)
                .map(|name| (name.to_string(), ToolOutputKind::Function))
        })
        .collect();

    Ok((
        json!({
            "model": model_info.slug,
            "messages": messages,
            "stream": true,
            "stream_options": {
                "include_usage": true,
            },
            "store": false,
            "max_completion_tokens": 384000,
            "tools": tools,
            "thinking": {
                "type": "enabled",
            },
            "reasoning_effort": "high",
            "temperature": 0.3,
        }),
        tool_kinds,
    ))
}

fn build_system_prompt(prompt: &Prompt) -> String {
    let cwd = prompt.cwd.as_deref().unwrap_or_else(|| Path::new("."));
    let date = chrono::Local::now().format("%Y-%m-%d");
    let mut system_prompt = format!(
        "{}\n\nCurrent date: {date}\nCurrent working directory: {cwd}",
        LITTLE_CODER_SYSTEM_PROMPT.trim_end(),
        cwd = cwd.display(),
    );
    if let Some(guidance) = selected_tool_guidance(prompt) {
        system_prompt.push_str("\n\n");
        system_prompt.push_str(guidance);
    }
    system_prompt
}

fn selected_tool_guidance(prompt: &Prompt) -> Option<&'static str> {
    let user_text = all_user_text(prompt);
    if user_text.is_empty() {
        return None;
    }
    let lower = user_text.to_lowercase();
    if lower.contains("browser")
        || lower.contains("javascript")
        || lower.contains("xss")
        || lower.contains("webpage")
        || lower.contains("url")
    {
        return Some(BROWSER_RESEARCH_TOOL_GUIDANCE);
    }
    Some(READ_WRITE_TOOL_GUIDANCE)
}

fn all_user_text(prompt: &Prompt) -> String {
    prompt
        .get_formatted_input()
        .iter()
        .fold(String::new(), |mut acc, item| {
            if let ResponseItem::Message { role, content, .. } = item
                && role == "user"
            {
                acc.push_str(&message_content(content));
            }
            acc
        })
}

fn message_content(content: &[ContentItem]) -> String {
    content
        .iter()
        .filter_map(|item| match item {
            ContentItem::InputText { text } | ContentItem::OutputText { text, .. } => {
                Some(text.as_str())
            }
            ContentItem::InputImage { .. } | ContentItem::InputAudio { .. } => None,
        })
        .collect()
}

fn build_tools() -> Vec<Value> {
    vec![
        pi::tool(
            "read",
            "Read the contents of a file. Supports text files and images (jpg, png, gif, webp). Images are sent as attachments. For text files, output is truncated to 2000 lines or 50KB (whichever is hit first). Use offset/limit for large files. When you need the full file, continue with offset until complete.",
            json!({"type":"object","required":["path"],"properties":{"path":{"type":"string","description":"Path to the file to read (relative or absolute)"},"offset":{"type":"number","description":"Line number to start reading from (1-indexed)"},"limit":{"type":"number","description":"Maximum number of lines to read"}}}),
        ),
        pi::tool(
            "bash",
            "Execute a bash command in the current working directory. Returns stdout and stderr. Output is truncated to last 2000 lines or 50KB (whichever is hit first). If truncated, full output is saved to a temp file. Optionally provide a timeout in seconds.",
            json!({"type":"object","required":["command"],"properties":{"command":{"type":"string","description":"Bash command to execute"},"timeout":{"type":"number","description":"Timeout in seconds (optional, no default timeout)"}}}),
        ),
        pi::tool(
            "edit",
            "Edit a single file using exact text replacement. Every edits[].oldText must match a unique, non-overlapping region of the original file. If two changes affect the same block or nearby lines, merge them into one edit instead of emitting overlapping edits. Do not include large unchanged regions just to connect distant changes.",
            json!({"type":"object","required":["path","edits"],"properties":{"path":{"type":"string","description":"Path to the file to edit (relative or absolute)"},"edits":{"type":"array","items":{"type":"object","required":["oldText","newText"],"properties":{"oldText":{"type":"string","description":"Exact text for one targeted replacement. It must be unique in the original file and must not overlap with any other edits[].oldText in the same call."},"newText":{"type":"string","description":"Replacement text for this targeted edit."}},"additionalProperties":false},"description":"One or more targeted replacements. Each edit is matched against the original file, not incrementally. Do not include overlapping or nested edits. If two changes touch the same block or nearby lines, merge them into one edit instead."}},"additionalProperties":false}),
        ),
        pi::tool(
            "write",
            "Write content to a file. Creates the file if it doesn't exist, overwrites if it does. Automatically creates parent directories.",
            json!({"type":"object","required":["path","content"],"properties":{"path":{"type":"string","description":"Path to the file to write (relative or absolute)"},"content":{"type":"string","description":"Content to write to the file"}}}),
        ),
        pi::tool(
            "BrowserNavigate",
            "Navigate the browser to a URL. Must start with http:// or https://.",
            json!({"type":"object","required":["url"],"properties":{"url":{"type":"string","description":"URL to navigate to"}}}),
        ),
        pi::tool(
            "BrowserClick",
            "Click an element by CSS selector, or by ARIA role (with optional accessible name).",
            json!({"type":"object","properties":{"selector":{"type":"string","description":"CSS selector"},"role":{"type":"string","description":"ARIA role (e.g. button, link)"},"name":{"type":"string","description":"Accessible name for role"}}}),
        ),
        pi::tool(
            "BrowserType",
            "Fill a form field by selector. Optionally submit by pressing Enter.",
            json!({"type":"object","required":["selector","text"],"properties":{"selector":{"type":"string","description":"CSS selector of the input"},"text":{"type":"string","description":"Text to type"},"submit":{"type":"boolean","description":"Press Enter after typing"}}}),
        ),
        pi::tool(
            "BrowserScroll",
            "Scroll the current page up or down by a pixel amount (default 800px down).",
            json!({"type":"object","properties":{"direction":{"type":"string","description":"up or down"},"amount":{"type":"integer","description":"Pixels (default 800)"}}}),
        ),
        pi::tool(
            "BrowserExtract",
            "Extract the current page's readable text. Returns a 2KB chunk with cursor+has_more so one page can't swamp context. Call repeatedly with the last 'next=' cursor.",
            json!({"type":"object","properties":{"cursor":{"type":"string","description":"Byte offset to start from (default 0)"}}}),
        ),
        pi::tool(
            "BrowserBack",
            "Navigate back in the browser's history.",
            json!({"type":"object","properties":{}}),
        ),
        pi::tool(
            "BrowserHistory",
            "List the last 20 URLs visited in this session.",
            json!({"type":"object","properties":{}}),
        ),
        pi::tool(
            "EvidenceAdd",
            "Save a short evidence snippet with its source and a one-line note. Use for any fact you will cite in your final answer. Snippet is capped at 1KB.",
            json!({"type":"object","required":["source","note","snippet"],"properties":{"source":{"type":"string","description":"URL or identifier of origin"},"note":{"type":"string","description":"One-line summary for later recall"},"snippet":{"type":"string","description":"The exact citable span (<=1KB)"}}}),
        ),
        pi::tool(
            "EvidenceGet",
            "Retrieve a previously-saved evidence entry by its id.",
            json!({"type":"object","required":["id"],"properties":{"id":{"type":"string","description":"Evidence id from EvidenceAdd/List"}}}),
        ),
        pi::tool(
            "EvidenceList",
            "List all evidence entries in this session: id, source, one-line note.",
            json!({"type":"object","properties":{}}),
        ),
        pi::tool(
            "glob",
            "Find files matching a glob pattern. Returns a sorted list of matching paths (up to 500). Common dependency/build/cache dirs (node_modules, .git, dist, …) are skipped, and the walk is bounded — for a focused search, pass a `path` rather than globbing a whole home directory.",
            json!({"type":"object","required":["pattern"],"properties":{"pattern":{"type":"string","description":"Glob pattern e.g. **/*.py"},"path":{"type":"string","description":"Base directory (default: cwd)"}}}),
        ),
        pi::tool(
            "webfetch",
            "Fetch a URL and return its text content (HTML stripped). Capped at 25K chars.",
            json!({"type":"object","required":["url"],"properties":{"url":{"type":"string","description":"URL to fetch"},"prompt":{"type":"string","description":"Hint for what to extract (informational)"}}}),
        ),
        pi::tool(
            "websearch",
            "Search the web via DuckDuckGo and return the top ~8 results as Markdown.",
            json!({"type":"object","required":["query"],"properties":{"query":{"type":"string","description":"Search query"}}}),
        ),
        pi::tool(
            "ShellSession",
            "Run a command in a persistent bash session. cd, env vars, and shell state persist across calls. One command per turn. Default timeout 30s (increase to 120-300 for installs/builds). Output is line-capped with head/tail truncation and a trailing [exit=N cwd=… timed_out=…] footer.",
            json!({"type":"object","required":["command"],"properties":{"command":{"type":"string","description":"Shell command to run"},"timeout":{"type":"integer","description":"Seconds (default 30, max 600)"}}}),
        ),
        pi::tool(
            "ShellSessionCwd",
            "Print the current working directory of the shell session.",
            json!({"type":"object","properties":{}}),
        ),
        pi::tool(
            "ShellSessionReset",
            "Kill and restart the bash session. Use only if it becomes unresponsive.",
            json!({"type":"object","properties":{}}),
        ),
    ]
}

const READ_WRITE_TOOL_GUIDANCE: &str = r#"## Tool Usage Guidance

### Read
## Read Tool
Read a file's contents with line numbers.

REQUIRED: path (absolute path)
OPTIONAL: limit (max lines), offset (start line, 0-indexed)

RULES:
- Always use absolute paths, never relative
- Use limit+offset for large files (read in chunks of 100-200 lines)
- Returns format: "N\tline_content" (tab-separated line number + content)

EXAMPLE:
```tool
{"name": "Read", "input": {"path": "/absolute/path/to/file.py"}}
```

EXAMPLE with range:
```tool
{"name": "Read", "input": {"path": "/absolute/path/to/file.py", "limit": 50, "offset": 100}}
```

### Write
## Write Tool
Create a **new** file with the given content. Creates parent directories automatically.

REQUIRED: path (absolute), content (full file content)

**Write is for creating new files only.** If the file already exists, Write will be **refused** by the tool and return an error telling you to use Edit instead. Do not retry Write on the same path — it will be refused again.

WHEN TO USE Write:
- The file does not exist yet and you are creating it from scratch

WHEN TO USE Edit INSTEAD:
- ANY change to an existing file — bug fixes, refactors, format tweaks, adding a function, renaming a variable, everything. Edit takes `path` + `edits: [{oldText, newText}]` and patches in place.
- Iterating after a failed test — never retype the whole file

If you need to completely replace an existing file's content, Edit can still do that: pass the entire current content as `oldText` and the full new content as `newText`. Read the file first if you don't already have its current content.

EXAMPLE:
```tool
{"name": "Write", "input": {"path": "/tmp/example/new_module.py", "content": "def hello():\n    return 'hi'\n"}}
```
NOTE: Always use the EXACT file path given in the task, never a placeholder.
"#;

const BROWSER_RESEARCH_TOOL_GUIDANCE: &str = r#"## Algorithm Reference

### Workspace Documentation
Before writing code for a non-trivial task, check if the workspace has a problem specification or convention document. These are cheap to read and often contain the exact format rules, edge cases, or constraints the tests assert — which the model would otherwise have to reverse-engineer from tests alone. Look for (in priority order): `.docs/instructions.md` and `.docs/instructions.append.md` (exercism-style problem specs), `AGENTS.md` / `CLAUDE.md` (agent-specific instructions at repo root), `README.md` in the current directory, `SPEC.md` / `SPECIFICATION.md`, and `docs/*.md`. Use Glob to discover them (`*.md`, `.docs/*.md`, `AGENTS.md`) and Read the relevant one. Do this ONCE at the start of a task, not every turn. If the spec disambiguates a failing test (e.g. "the first and last elements must match" or "spaces and punctuation are excluded"), that single read saves many debug iterations. Skip for pure read-only questions — only invest the Read call when you are about to change code.


## Tool Usage Guidance

### Glob
## Glob Tool
Find files matching a glob pattern.

REQUIRED: pattern (glob pattern like "**/*.py")
OPTIONAL: path (directory to search in, defaults to cwd)

RULES:
- Use ** for recursive matching across directories
- Returns sorted list of matching file paths
- Good for finding files by extension or name pattern

EXAMPLE:
```tool
{"name": "Glob", "input": {"pattern": "**/*.py"}}
```

EXAMPLE with path:
```tool
{"name": "Glob", "input": {"pattern": "*.md", "path": "/path/to/docs/"}}
```

### Read
## Read Tool
Read a file's contents with line numbers.

REQUIRED: path (absolute path)
OPTIONAL: limit (max lines), offset (start line, 0-indexed)

RULES:
- Always use absolute paths, never relative
- Use limit+offset for large files (read in chunks of 100-200 lines)
- Returns format: "N\tline_content" (tab-separated line number + content)

EXAMPLE:
```tool
{"name": "Read", "input": {"path": "/absolute/path/to/file.py"}}
```

EXAMPLE with range:
```tool
{"name": "Read", "input": {"path": "/absolute/path/to/file.py", "limit": 50, "offset": 100}}
```

### Write
## Write Tool
Create a **new** file with the given content. Creates parent directories automatically.

REQUIRED: path (absolute), content (full file content)

**Write is for creating new files only.** If the file already exists, Write will be **refused** by the tool and return an error telling you to use Edit instead. Do not retry Write on the same path — it will be refused again.

WHEN TO USE Write:
- The file does not exist yet and you are creating it from scratch

WHEN TO USE Edit INSTEAD:
- ANY change to an existing file — bug fixes, refactors, format tweaks, adding a function, renaming a variable, everything. Edit takes `path` + `edits: [{oldText, newText}]` and patches in place.
- Iterating after a failed test — never retype the whole file

If you need to completely replace an existing file's content, Edit can still do that: pass the entire current content as `oldText` and the full new content as `newText`. Read the file first if you don't already have its current content.

EXAMPLE:
```tool
{"name": "Write", "input": {"path": "/tmp/example/new_module.py", "content": "def hello():\n    return 'hi'\n"}}
```
NOTE: Always use the EXACT file path given in the task, never a placeholder.

## Research-first directive
This task involves online research. Before producing a final answer:
1. Use BrowserNavigate / BrowserExtract (or WebSearch for first hops) to gather facts.
2. Save each citable fact via EvidenceAdd before relying on it.
3. Only after evidence is in place should you consider any Edit/Write tool calls.
Skipping the gather step (going straight to Edit/Write or guessing from memory) is wrong — restart with the browse step instead.
"#;
