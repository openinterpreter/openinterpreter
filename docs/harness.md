---
title: Harness
description: Harness ids, route compatibility, and what each mode changes.
---

Harness mode is an Open Interpreter addition. It changes the model-facing
prompt, tool schema, message conversion, and response handling while keeping
the native Open Interpreter runtime.

The public runtime does not shell out to the real external agent CLI.

## Configure

```toml
harness = "kimi-code"
harness_guidance = true
```

For one run:

```bash
interpreter -c harness='"kimi-code"' "solve this task"
```

## Harness IDs

| Harness ID | Wire API | Request route | Reference |
| --- | --- | --- | --- |
| unset or `""` | `responses` | Responses API | Native Open Interpreter/Codex-compatible surface |
| unset or `""` | `chat` | Chat Completions compatibility | Generic OpenAI-compatible chat providers |
| `claude-code` | `responses`, `chat`, or `messages` | Claude Code shaping over the provider transport | [Claude Code](https://docs.anthropic.com/en/docs/claude-code/overview) full agent surface |
| `claude-code-bare` | `responses`, `chat`, or `messages` | Claude Code shaping over the provider transport | [Claude Code](https://docs.anthropic.com/en/docs/claude-code/overview) bare profile |
| `zcode` | `messages` | Anthropic Messages harness | ZCode-shaped GLM coding-agent surface |
| `deepseek-tui` | `chat` | Chat harness | [DeepSeek TUI](https://github.com/DeepSeek-TUI/DeepSeek-TUI) / [CodeWhale](https://www.codewhale.ai/) |
| `kimi-code` | `chat` | Chat harness | Current [Kimi Code](https://www.kimi.com/code/docs/en/) / [GitHub](https://github.com/MoonshotAI/kimi-code) profile |
| `kimi-cli` | `chat` | Chat harness | Legacy [Kimi CLI](https://moonshotai.github.io/kimi-cli/) / [GitHub](https://github.com/MoonshotAI/kimi-cli) profile |
| `qwen-code` | `chat` | Chat harness | [Qwen Code](https://qwenlm.github.io/qwen-code-docs/en/cli/index) / [GitHub](https://github.com/QwenLM/qwen-code) |
| `swe-agent` | `chat` | Chat harness | [SWE-agent](https://swe-agent.com/) / [GitHub](https://github.com/SWE-agent/SWE-agent) |
| `minimal` | `chat` | Chat harness | Open Interpreter minimal chat-tool surface |
| any other string | `chat` | Chat Completions compatibility | Custom marker; no built-in harness request builder |

## Reference Projects

These links are the external products or repositories whose user-facing
surfaces informed the corresponding harness modes:

- [Claude Code](https://docs.anthropic.com/en/docs/claude-code/overview)
  from Anthropic.
- ZCode, a GLM coding-agent surface used with Anthropic Messages-compatible
  endpoints such as Z.AI's Coding Plan endpoint.
- [DeepSeek TUI](https://github.com/DeepSeek-TUI/DeepSeek-TUI), with product
  context at [CodeWhale](https://www.codewhale.ai/).
- Current [Kimi Code](https://www.kimi.com/code/docs/en/) and its
  [MoonshotAI/kimi-code](https://github.com/MoonshotAI/kimi-code) repository.
- The legacy Python [Kimi CLI](https://moonshotai.github.io/kimi-cli/) and its
  [MoonshotAI/kimi-cli](https://github.com/MoonshotAI/kimi-cli) repository.
- [Qwen Code CLI](https://qwenlm.github.io/qwen-code-docs/en/cli/index) and
  its [QwenLM/qwen-code](https://github.com/QwenLM/qwen-code) repository.
- [SWE-agent](https://swe-agent.ai/) and its
  [SWE-agent/SWE-agent](https://github.com/SWE-agent/SWE-agent) repository.

## Route Compatibility

Harness routing is strict:

| Provider `wire_api` | Compatible harnesses |
| --- | --- |
| `responses` | Native Responses, `claude-code`, and `claude-code-bare`. |
| `chat` | Native chat compatibility, `claude-code`, `claude-code-bare`, `deepseek-tui`, `kimi-code`, `kimi-cli`, `qwen-code`, `swe-agent`, and `minimal`. |
| `messages` | `claude-code`, `claude-code-bare`, and `zcode`. Native mode is rejected because Messages requires a harness-native transport. |

This means an Anthropic-style provider normally needs:

```toml
model_provider = "anthropic"
harness = "claude-code"
```

Most OpenAI-compatible hosted providers use `wire_api = "chat"` and can either
run through generic chat compatibility or through a matching chat harness.

## Automatic Harness Defaults

When the config does not set `harness`, Open Interpreter may infer one from
the selected provider/model:

| Detected family | Default harness |
| --- | --- |
| Anthropic, Claude model ids, Anthropic base URL, or any `messages` provider | `claude-code` |
| Kimi/Moonshot provider ids, names, base URLs, or model ids | `kimi-code` |
| Qwen/QwQ/DashScope provider ids, names, base URLs, or model ids | `qwen-code` |
| DeepSeek provider ids, names, base URLs, or model ids | `claude-code-bare` |

Explicit `harness = "..."` always wins.

## What Each Harness Changes

### `claude-code`

Builds Claude Code-shaped requests over the selected provider's Responses,
Chat, or Anthropic Messages transport. It adds a Claude Code system prompt,
thinking configuration, context-management settings, title-generation
requests, and a Claude-shaped tool surface. Transport-specific headers and
message conversion are applied when the provider requires them.

It maps supported tools into Anthropic tool definitions and includes handlers
for Bash/PowerShell, Read, Write, Edit, TodoWrite, Glob, Grep, web search/fetch,
LSP, scheduled wakeups, and Claude-style subagents.

### `claude-code-bare`

Uses the same provider transport as `claude-code`, but with the bare Claude
Code profile. The bare profile uses a smaller prompt/profile shape and
different output config defaults. DeepSeek selects this profile automatically.

### `zcode`

Uses Anthropic Messages-compatible requests with a ZCode-shaped system prompt,
headers, tools, todo and plan behavior, skills, session context, and subagent
surface. Tool calls still run inside Open Interpreter's native Rust runtime.

The harness requires `wire_api = "messages"`. The built-in Z.AI and Z.AI
Coding Plan providers use `wire_api = "chat"`, so use the Messages endpoint
configuration in [Z.AI, GLM, and ZCode](/docs/zai-glm) when you want the actual
ZCode route.

### `kimi-code`

Uses Chat Completions-compatible requests with the current Kimi Code prompt,
tool definitions, prompt-cache key, thinking configuration, and message
format. Supported tool calls are executed by Open Interpreter's native Rust
runtime; it does not invoke the external Kimi executable.

This is the default for Kimi and Moonshot providers, including Kimi K3. Kimi
Code subscriptions use the `kimi-for-coding` provider and can authenticate
through the built-in Kimi sign-in flow. Moonshot Platform API keys use the
`moonshotai` provider.

### `kimi-cli`

Uses Chat Completions-compatible requests with the legacy Python Kimi CLI-shaped system prompt.
It includes working-directory listing, AGENTS.md loading, Kimi skill discovery,
prompt-cache keys, reasoning-effort mapping, and Kimi tool schemas.

Kimi tool handlers include Shell, ReadFile, WriteFile, StrReplaceFile, Glob,
Grep, ReadMediaFile, SearchWeb, FetchUrl, SetTodoList, plan-mode controls,
background task list/output/stop, AskUserQuestion, and Agent.

Keep this mode only when you specifically need compatibility with the retired
Python CLI profile. New Kimi sessions should use `kimi-code`.

### `deepseek-tui`

Uses Chat Completions-compatible requests with a DeepSeek TUI/CodeWhale-shaped
system prompt. It adds turn metadata, repository context, generated project
instructions when none are found, and DeepSeek TUI tool schemas.

DeepSeek TUI tool handlers include shell, apply patch, edit/write/read file,
list directory, grep/file search, git status/diff, diagnostics, checklist,
plan, and tool search.

### `qwen-code`

Uses Chat Completions-compatible requests with Qwen Code startup context. It
adds a synthetic setup exchange containing date, OS, current directory, and a
small folder listing before the user conversation.

Qwen handlers include read file, write file, edit, shell command, glob, grep,
todo write, ask user question, plan exit, and agent.

### `swe-agent`

Uses a SWE-agent-style discussion/command loop rather than tool schemas. The
assistant response is parsed for a shell command, Open Interpreter injects the
corresponding action, and command output returns as an observation. The default
command timeout is 30 seconds.

### `minimal`

Uses a compact software-agent system prompt and maps function tools into a
plain Chat Completions tool list. It is useful when a provider supports chat
tools but does not need a provider-specific harness surface.

## Guidance

`harness_guidance = true` is enabled by default. Today it only adds extra
guidance for `kimi-cli`; other harnesses ignore it.

Disable it for a stricter harness-shaped run:

```toml
harness_guidance = false
```

## Related Config

```toml
model_provider = "moonshotai"
model = "kimi-k3"
harness = "kimi-code"

[model_providers.moonshotai]
name = "Moonshot AI"
base_url = "https://api.moonshot.ai/v1"
env_key = "MOONSHOT_API_KEY"
wire_api = "chat"
```

For provider and wire-api details, see [Providers](/docs/providers).
