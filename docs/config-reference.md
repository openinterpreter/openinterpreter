---
title: Config Reference
description: Important config.toml keys and tables.
---

This reference lists the Open Interpreter settings users most often need. The
source of truth for every field is the generated schema in
`codex-rs/core/config.schema.json`.

## Top-Level Keys

| Key                           | Values                                                    | Purpose                                       |
| ----------------------------- | --------------------------------------------------------- | --------------------------------------------- |
| `model`                       | string                                                    | Default model id.                             |
| `model_provider`              | provider id                                               | Provider entry used for `model`.              |
| `model_reasoning_effort`      | `minimal`, `low`, `medium`, `high`, `xhigh`               | Reasoning budget for supported models.        |
| `model_reasoning_summary`     | `auto`, `concise`, `detailed`, `none`                     | How much reasoning summary to show.           |
| `model_verbosity`             | `low`, `medium`, `high`                                   | Verbosity for supported Responses API models. |
| `approval_policy`             | `untrusted`, `on-request`, `never`                        | When to ask before running commands.          |
| `approvals_reviewer`          | `user`, `auto_review`                                     | Who reviews eligible approval prompts.        |
| `sandbox_mode`                | `read-only`, `workspace-write`, `danger-full-access`      | Local command sandbox.                        |
| `default_permissions`         | profile name                                              | Selects a beta permissions profile.           |
| `personality`                 | `friendly`, `pragmatic`, `none`                           | TUI communication style.                      |
| `web_search`                  | `cached`, `live`, `disabled`                              | Web search behavior.                          |
| `log_dir`                     | path                                                      | Directory for logs.                           |
| `file_opener`                 | `vscode`, `vscode-insiders`, `windsurf`, `cursor`, `none` | Editor used for file citations.               |
| `harness`                     | string                                                    | Open Interpreter harness compatibility mode.  |
| `harness_guidance`            | boolean                                                   | Allow OI guidance inside harness mode.        |
| `check_for_update_on_startup` | boolean                                                   | Enable managed standalone update checks.      |

## Feature Flags

| Key                       | Default posture             | Purpose                           |
| ------------------------- | --------------------------- | --------------------------------- |
| `features.apps`           | off                         | App/connectors surface.           |
| `features.plugins`        | off                         | Plugin bundles.                   |
| `features.hooks`          | on                          | Lifecycle hooks.                  |
| `features.memories`       | off                         | Persistent memory generation/use. |
| `features.multi_agent`    | on                          | Subagent tools and `/agent`.      |
| `features.shell_tool`     | on                          | Built-in shell command tool.      |
| `features.shell_snapshot` | on                          | Shell environment snapshotting.   |
| `features.unified_exec`   | on except where unsupported | PTY-backed exec tool.             |
| `features.undo`           | off                         | Undo support where available.     |
| `features.network_proxy`  | off                         | Sandboxed network proxy controls. |

## Provider Tables

```toml
[model_providers.example]
name = "Example"
base_url = "https://api.example.com/v1"
env_key = "EXAMPLE_API_KEY"
wire_api = "responses"
request_max_retries = 4
stream_max_retries = 5
stream_idle_timeout_ms = 300000
```

Provider auth can also be command-backed:

```toml
[model_providers.example.auth]
command = "example-token"
args = ["print"]
refresh_interval_ms = 300000
timeout_ms = 5000
```

## Sandbox Tables

```toml
[sandbox_workspace_write]
network_access = false
exclude_tmpdir_env_var = false
exclude_slash_tmp = false
writable_roots = ["/tmp/project-cache"]
```

For finer-grained access, use permissions profiles instead of mixing both
systems in one active config.

## Permission Profiles

```toml
default_permissions = "project-edit"

[permissions.project-edit.filesystem]
":minimal" = "read"

[permissions.project-edit.filesystem.":workspace_roots"]
"." = "write"
"**/*.env" = "deny"

[permissions.project-edit.network]
enabled = true

[permissions.project-edit.network.domains]
"api.openai.com" = "allow"
"*.github.com" = "allow"
```

See [Permissions](/docs/permissions) for the complete model.

## MCP Tables

```toml
[mcp_servers.docs]
command = "docs-mcp"
args = ["--stdio"]
env = { DOCS_TOKEN = "env:DOCS_TOKEN" }
startup_timeout_sec = 10
tool_timeout_sec = 60
required = false
enabled_tools = ["search", "read"]
disabled_tools = ["delete"]
default_tools_approval_mode = "prompt"

[mcp_servers.docs.tools.search]
approval_mode = "approve"
```

HTTP MCP servers use `url`, `bearer_token_env_var`, `http_headers`, and
`env_http_headers`.

## Hooks

Hooks can be configured inline:

```toml
[[hooks.PreToolUse]]
matcher = "^Bash$"

[[hooks.PreToolUse.hooks]]
type = "command"
command = "python3 .openinterpreter/hooks/pre_tool_use.py"
timeout = 30
statusMessage = "Checking command"
```

Or in `hooks.json` next to the active config layer. See [Hooks](/docs/hooks).

## Agents

```toml
[agents]
max_threads = 6
max_depth = 1
job_max_runtime_seconds = 1800

[agents.explorer]
description = "Investigate code and report findings without editing."
model_reasoning_effort = "medium"
sandbox_mode = "read-only"
```

See [Subagents](/docs/subagents).
