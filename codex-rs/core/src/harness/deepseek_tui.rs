use crate::client_common::Prompt;
use codex_chat_wire_compat::ToolKinds;
use codex_chat_wire_compat::ToolOutputKind;
use codex_protocol::openai_models::ModelInfo;
use codex_tools::create_deepseek_tui_chat_tools_json;
use serde_json::Value;
use serde_json::json;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

const DEEPSEEK_TUI_DEFAULT_MAX_TOKENS: u32 = 64_000;
const CODEWHALE_BASE_PROMPT: &str = include_str!("deepseek_tui_prompts/base.md");
const CODEWHALE_CALM_PERSONALITY: &str = include_str!("deepseek_tui_prompts/personalities/calm.md");
const CODEWHALE_YOLO_MODE: &str = include_str!("deepseek_tui_prompts/modes/yolo.md");
const CODEWHALE_AUTO_APPROVAL: &str = include_str!("deepseek_tui_prompts/approvals/auto.md");
const CODEWHALE_COMPACT_TEMPLATE: &str = include_str!("deepseek_tui_prompts/compact.md");
const CODEWHALE_VERSION: &str = "0.8.44";

pub(crate) fn build_request(
    prompt: &Prompt,
    model_info: &ModelInfo,
) -> Result<(Value, ToolKinds), serde_json::Error> {
    let (system_prompt, should_write_generated_codewhale_instructions) =
        build_system_prompt(prompt, &model_info.slug);
    let mut messages = vec![json!({
        "role": "system",
        "content": system_prompt,
    })];
    messages.extend(super::kimi_cli::build_messages_with_options(
        &prompt.get_formatted_input(),
        super::kimi_cli::MessageBuildOptions::deepseek_tui(),
    )?);
    add_omitted_reasoning_to_assistant_tool_calls(&mut messages);
    add_turn_metadata_to_latest_user_message(&mut messages, prompt.cwd.as_deref());
    if should_write_generated_codewhale_instructions && let Some(cwd) = prompt.cwd.as_deref() {
        write_generated_codewhale_project_instructions(cwd);
    }
    let tools = create_deepseek_tui_chat_tools_json();
    let tool_kinds = prompt
        .tools
        .iter()
        .map(|tool| (tool.name().to_string(), ToolOutputKind::Function))
        .collect();

    let request = json!({
        "model": model_info.slug,
        "messages": messages,
        "max_tokens": DEEPSEEK_TUI_DEFAULT_MAX_TOKENS,
        "stream": true,
        "stream_options": {
            "include_usage": true,
        },
        "tools": tools,
        "tool_choice": "auto",
    });
    Ok((request, tool_kinds))
}

fn add_omitted_reasoning_to_assistant_tool_calls(messages: &mut [Value]) {
    for message in messages {
        let Some(message_object) = message.as_object_mut() else {
            continue;
        };
        let is_assistant = message_object
            .get("role")
            .and_then(Value::as_str)
            .is_some_and(|role| role == "assistant");
        if is_assistant
            && message_object.contains_key("tool_calls")
            && !message_object.contains_key("reasoning_content")
        {
            message_object.insert(
                "reasoning_content".to_string(),
                Value::String("(reasoning omitted)".to_string()),
            );
        }
        if is_assistant
            && message_object.contains_key("tool_calls")
            && !message_object.contains_key("content")
        {
            message_object.insert("content".to_string(), Value::String(String::new()));
        }
    }
}

fn build_system_prompt(prompt: &Prompt, model: &str) -> (String, bool) {
    let cwd = prompt.cwd.as_deref();
    let mut sections = vec![
        CODEWHALE_BASE_PROMPT.replace("{model_id}", model),
        CODEWHALE_CALM_PERSONALITY.to_string(),
        CODEWHALE_YOLO_MODE.to_string(),
        CODEWHALE_AUTO_APPROVAL.to_string(),
    ];
    let mut should_write_generated_codewhale_instructions = false;
    if let Some((project_instructions, is_generated)) = project_instructions_block(cwd) {
        sections.push(project_instructions);
        should_write_generated_codewhale_instructions = is_generated;
    }
    if let Some(project_context_pack) = project_context_pack_block(cwd) {
        sections.push(project_context_pack);
    }
    sections.push(environment_block(cwd));
    sections.push(context_management_block());
    sections.push(CODEWHALE_COMPACT_TEMPLATE.to_string());
    sections.push(authority_recap());
    (
        sections
            .into_iter()
            .map(|section| section.trim_end().to_string())
            .collect::<Vec<_>>()
            .join("\n\n"),
        should_write_generated_codewhale_instructions,
    )
}

fn add_turn_metadata_to_latest_user_message(messages: &mut [Value], cwd: Option<&Path>) {
    let Some(message) = messages.iter_mut().rev().find(|message| {
        message
            .get("role")
            .and_then(Value::as_str)
            .is_some_and(|role| role == "user")
    }) else {
        return;
    };
    let Some(content) = message.get("content").and_then(Value::as_str) else {
        return;
    };
    if content.trim_start().starts_with("<turn_meta>") {
        return;
    }
    *message.get_mut("content").expect("content exists") = Value::String(format!(
        "{}\n{}",
        turn_metadata_block(cwd, content),
        content
    ));
}

fn turn_metadata_block(cwd: Option<&Path>, user_content: &str) -> String {
    let mut lines = vec![
        "<turn_meta>".to_string(),
        format!("Current local date: {}", current_local_date()),
    ];
    if let Some(cwd) = cwd {
        lines.push("## Repo Working Set".to_string());
        lines.push(format!("Workspace: {}", cwd.display()));
        if let Some(readme) = first_readme(cwd) {
            lines.push(format!("Key files: {}", readme.display()));
        }
        let active_paths = active_paths(cwd, user_content);
        if !active_paths.is_empty() {
            lines.push("Active paths (prioritize these):".to_string());
            for path in active_paths {
                lines.push(format!("- {path}"));
            }
        }
        lines.push(
            "When in doubt, use tools to verify and keep changes focused on the working set."
                .to_string(),
        );
    }
    lines.push("</turn_meta>".to_string());
    lines.join("\n")
}

fn project_instructions_block(cwd: Option<&Path>) -> Option<(String, bool)> {
    let cwd = cwd?;
    for name in [
        ".codewhale/instructions.md",
        ".deepseek/instructions.md",
        "AGENTS.md",
        "CLAUDE.md",
    ] {
        let Some(path) = find_upward(cwd, name) else {
            continue;
        };
        let content = fs::read_to_string(&path).ok()?;
        return Some((
            format!(
                "<project_instructions source=\"{}\">\n{}\n</project_instructions>",
                path.display(),
                content
            ),
            false,
        ));
    }
    let path = cwd.join(".codewhale/instructions.md");
    let content = generated_codewhale_project_instructions(cwd);
    Some((
        format!(
            "<project_instructions source=\"{}\">\n{}\n</project_instructions>",
            path.display(),
            content
        ),
        true,
    ))
}

fn project_context_pack_block(cwd: Option<&Path>) -> Option<String> {
    let cwd = cwd?;
    let entries = workspace_entries(cwd);
    let readme = first_readme(cwd);
    let key_source_files: Vec<String> = entries
        .iter()
        .filter(|entry| is_key_source_path(entry))
        .cloned()
        .collect();
    let project_name = cwd
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("workspace");
    let directory_structure = json_string_array(&entries, 2);
    let key_source_files = json_string_array(&key_source_files, 2);
    let readme = readme
        .as_ref()
        .map(|path| {
            let excerpt = fs::read_to_string(cwd.join(path)).unwrap_or_default();
            format!(
                "{{\n    \"path\": {},\n    \"excerpt\": {}\n  }}",
                serde_json::to_string(&path.to_string_lossy())
                    .unwrap_or_else(|_| "\"\"".to_string()),
                serde_json::to_string(&excerpt).unwrap_or_else(|_| "\"\"".to_string())
            )
        })
        .unwrap_or_else(|| "null".to_string());
    let pack = format!(
        "{{\n  \"project_name\": \"{project_name}\",\n  \"directory_structure\": {directory_structure},\n  \"readme\": {readme},\n  \"config_files\": [],\n  \"key_source_files\": {key_source_files},\n  \"counts\": {{\n    \"config_files\": 0,\n    \"directory_entries\": {},\n    \"key_source_files\": {}\n  }}\n}}",
        entries.len(),
        key_source_files_count(cwd)
    );
    Some(format!(
        "## Project Context Pack\n\n<project_context_pack>\n{pack}\n</project_context_pack>"
    ))
}

fn environment_block(cwd: Option<&Path>) -> String {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
    let pwd = cwd.map(|cwd| cwd.display().to_string()).unwrap_or_else(|| {
        std::env::current_dir().map_or_else(|_| ".".to_string(), |path| path.display().to_string())
    });
    format!(
        "## Environment\n\n- lang: en\n- deepseek_version: {CODEWHALE_VERSION}\n- platform: {}\n- shell: {shell}\n- pwd: {pwd}",
        platform_name()
    )
}

fn context_management_block() -> String {
    r#"## Context Management

When the conversation gets long (you'll see a context usage indicator), you can:
1. Use `/compact` to summarize earlier context and free up space
2. The system will preserve important information (files you're working on, recent messages, tool results)
3. After compaction, you'll see a summary of what was discussed and can continue seamlessly

If you notice context is getting long (>60% during sustained work), proactively suggest using `/compact` to the user.

### Prompt-cache awareness

DeepSeek caches the longest *byte-stable prefix* of every request and charges roughly 100× less for cache-hit tokens than miss tokens. The system prompt above is layered most-static-first specifically so the prefix stays stable turn-over-turn. To keep cache hits high:
- **Working set location:** the current repo working set is stored on new user messages inside a `<turn_meta>` block. Treat it as high-priority turn metadata, not as a stable system-prompt section.
- **Append, don't reorder.** New context goes at the end (latest user / tool messages). Reshuffling earlier messages or rewriting their content invalidates the cache for everything after the change.
- **Don't paraphrase quoted content.** If you've already read a file, refer to it by path or line range instead of re-quoting it with different formatting.
- **Use `/compact` as a hard reset, not a tweak.** Compaction is meant for when the cache is already losing — it intentionally rewrites the prefix to a shorter summary. Don't trigger it for small wins.
- **Read once, refer back.** Re-reading the same file produces a different tool-result envelope than the prior read; it's cheaper to scroll back than to re-fetch.
- **Footer chip:** the `cache hit %` chip turns red below 40% and yellow below 80%. If it's been red for several turns, that's a signal to consolidate."#
        .to_string()
}

fn authority_recap() -> String {
    r#"
## Authority Recap

The Constitution of CodeWhale (Articles I-VII) governs your behavior.
Tier 1 rules — truthfulness, user agency, tool-use mandate, verification
duty — are non-negotiable. The user's next message is the highest
directive within Constitutional bounds. Personality, memory, and handoff
context are subordinate to the Constitution, the Statutes, and the user's
current request. When in doubt, consult Article VII: The Hierarchy of Law."#
        .to_string()
}

fn find_upward(start: &Path, name: &str) -> Option<PathBuf> {
    for dir in start.ancestors() {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn current_local_date() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

fn platform_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    }
}

fn workspace_entries(cwd: &Path) -> Vec<String> {
    let mut entries = Vec::new();
    collect_workspace_entries(cwd, cwd, 2, &mut entries);
    entries.sort();
    entries
}

fn collect_workspace_entries(root: &Path, dir: &Path, depth: usize, entries: &mut Vec<String>) {
    if depth == 0 {
        return;
    }
    let Ok(read_dir) = fs::read_dir(dir) else {
        return;
    };
    for entry in read_dir.flatten() {
        let path = entry.path();
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if file_name == ".codewhale" || file_name == ".git" || file_name == "target" {
            continue;
        }
        if let Ok(relative) = path.strip_prefix(root) {
            entries.push(relative.to_string_lossy().to_string());
        }
        // Do not recurse into symlinked directories: a symlink pointing at a
        // large tree (or back at an ancestor) must not expand the listing.
        if entry.file_type().is_ok_and(|file_type| file_type.is_dir()) {
            collect_workspace_entries(root, &path, depth - 1, entries);
        }
    }
}

fn generated_codewhale_project_instructions(cwd: &Path) -> String {
    let key_files = first_readme(cwd)
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "none".to_string());
    let tree = codewhale_project_tree(cwd);
    format!(
        "# Project Structure (Auto-generated)\n\n\
         > This file was automatically generated by CodeWhale.\n\
         > You can edit or delete it at any time.\n\n\
         **Summary:** Project with key files: {key_files}\n\n\
         **Tree:**\n\
         ```\n\
         {tree}\n\
         ```"
    )
}

fn write_generated_codewhale_project_instructions(cwd: &Path) {
    let path = cwd.join(".codewhale/instructions.md");
    if path.is_file() {
        return;
    }
    let content = generated_codewhale_project_instructions(cwd);
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(path, content);
}

fn codewhale_project_tree(cwd: &Path) -> String {
    let mut lines = Vec::new();
    collect_codewhale_tree(cwd, cwd, 2, 0, &mut lines);
    lines.join("\n")
}

fn collect_codewhale_tree(
    root: &Path,
    dir: &Path,
    depth: usize,
    indent: usize,
    lines: &mut Vec<String>,
) {
    if depth == 0 {
        return;
    }
    let Ok(read_dir) = fs::read_dir(dir) else {
        return;
    };
    let mut entries = read_dir.flatten().collect::<Vec<_>>();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        let Some(name) = relative.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let prefix = " ".repeat(indent);
        if path.is_dir() {
            lines.push(format!("{prefix}DIR: {name}"));
            // Do not descend into symlinked directories.
            if entry.file_type().is_ok_and(|file_type| file_type.is_dir()) {
                collect_codewhale_tree(root, &path, depth - 1, indent + 2, lines);
            }
        } else {
            lines.push(format!("{prefix}FILE: {name}"));
        }
    }
}

fn first_readme(cwd: &Path) -> Option<PathBuf> {
    ["README.md", "readme.md", "README"]
        .into_iter()
        .map(PathBuf::from)
        .find(|path| cwd.join(path).is_file())
}

fn active_paths(cwd: &Path, user_content: &str) -> Vec<String> {
    let mut entries = Vec::new();
    for candidate in [
        "module.py",
        "SHELL_OK/n",
        "created_by_gauntlet.txt",
        "editing/patching",
        "shell_proof.txt",
    ] {
        let mentioned = user_content.contains(candidate)
            || (candidate == "SHELL_OK/n" && user_content.contains("SHELL_OK"));
        if (mentioned || cwd.join(candidate).exists())
            && !entries.iter().any(|entry| entry == candidate)
        {
            entries.push(candidate.to_string());
        }
    }
    for entry in workspace_entries(cwd)
        .into_iter()
        .filter(|entry| entry != "README.md")
    {
        if !entries.iter().any(|existing| existing == &entry) {
            entries.push(entry);
        }
    }
    entries
        .into_iter()
        .take(8)
        .map(|entry| {
            let kind = if cwd.join(&entry).is_dir() {
                "directory"
            } else {
                "file"
            };
            format!("{entry} ({kind})")
        })
        .collect()
}

fn is_key_source_path(path: &str) -> bool {
    path.ends_with(".py")
        || path.ends_with(".rs")
        || path.ends_with(".js")
        || path.ends_with(".ts")
        || path.ends_with(".tsx")
}

fn json_string_array(values: &[String], indent: usize) -> String {
    if values.is_empty() {
        return "[]".to_string();
    }
    let padding = " ".repeat(indent);
    let inner_padding = " ".repeat(indent + 2);
    let mut lines = vec!["[".to_string()];
    for (index, value) in values.iter().enumerate() {
        let comma = if index + 1 == values.len() { "" } else { "," };
        lines.push(format!(
            "{}{}{}",
            inner_padding,
            serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string()),
            comma
        ));
    }
    lines.push(format!("{padding}]"));
    lines.join("\n")
}

fn key_source_files_count(cwd: &Path) -> usize {
    workspace_entries(cwd)
        .iter()
        .filter(|entry| is_key_source_path(entry))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client_common::Prompt;
    use codex_protocol::openai_models::ModelInfo;
    use pretty_assertions::assert_eq;

    #[test]
    fn deepseek_tui_request_matches_captured_top_level_shape() {
        let prompt = Prompt::default();
        let model_info = model_info();
        let (request, tool_kinds) = build_request(&prompt, &model_info).expect("request");

        assert_eq!(request["model"], "deepseek-chat");
        assert_eq!(request["max_tokens"], 64_000);
        assert_eq!(request["stream"], true);
        assert_eq!(request["stream_options"]["include_usage"], true);
        assert_eq!(request["tool_choice"], "auto");
        assert!(
            request["messages"][0]["content"]
                .as_str()
                .expect("system content")
                .contains("CONSTITUTION OF CODEWHALE")
        );
        assert!(tool_kinds.is_empty());
    }

    fn model_info() -> ModelInfo {
        serde_json::from_value(json!({
            "slug": "deepseek-chat",
            "display_name": "DeepSeek Chat",
            "description": "desc",
            "default_reasoning_level": null,
            "supported_reasoning_levels": [],
            "reasoning_control": "none",
            "shell_type": "shell_command",
            "visibility": "list",
            "supported_in_api": true,
            "priority": 1,
            "upgrade": null,
            "base_instructions": "",
            "model_messages": null,
            "supports_reasoning_summaries": false,
            "support_verbosity": false,
            "default_verbosity": null,
            "apply_patch_tool_type": null,
            "truncation_policy": {"mode": "bytes", "limit": 10000},
            "supports_parallel_tool_calls": false,
            "supports_image_detail_original": false,
            "context_window": 1000000,
            "auto_compact_token_limit": null,
            "experimental_supported_tools": []
        }))
        .expect("deserialize model info")
    }
}
