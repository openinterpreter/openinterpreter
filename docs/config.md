---
title: Configuration
description: Configure models, providers, approvals, sandboxing, features, MCP, and profiles.
---

Open Interpreter reads durable settings from TOML files. User-level config lives
at:

```text
~/.openinterpreter/config.toml
```

Project config can live in a trusted project under:

```text
.openinterpreter/config.toml
```

Command-line overrides use `-c key=value` and apply only to that invocation.

## Precedence

Higher-precedence values override lower-precedence values:

1. Built-in defaults
2. System or managed configuration
3. User config
4. Trusted project config
5. Selected profile
6. CLI overrides from `-c`, `--enable`, `--disable`, or dedicated flags

Use `/debug-config` in the TUI to inspect the effective values and where they
came from.

## Common Settings

```toml
model = "gpt-5.1-codex"
model_provider = "openai"

# "minimal" | "low" | "medium" | "high" | "xhigh"
model_reasoning_effort = "medium"

# "auto" | "concise" | "detailed" | "none"
model_reasoning_summary = "auto"

# "read-only" | "workspace-write" | "danger-full-access"
sandbox_mode = "workspace-write"

# "untrusted" | "on-request" | "never"
approval_policy = "on-request"

# "friendly" | "pragmatic" | "none"
personality = "pragmatic"

web_search = "cached"
log_dir = "~/.openinterpreter/log"
```

## Profiles

Profiles are named groups of settings:

```toml
[profiles.fast]
model = "gpt-5.1-codex-mini"
model_reasoning_effort = "low"

[profiles.review]
model = "gpt-5.1-codex"
model_reasoning_effort = "high"
sandbox_mode = "read-only"
```

Use one with:

```bash
interpreter --profile review
```

## CLI Overrides

`-c` accepts TOML-like values. Quote strings so your shell does not strip them:

```bash
interpreter -c model='"gpt-5.1-codex-mini"' -c approval_policy='"never"'
```

Feature flags also have short forms:

```bash
interpreter --enable hooks --disable memories
```

## Feature Flags

Optional behavior lives under `[features]`.

```toml
[features]
hooks = true
multi_agent = true
shell_tool = true
shell_snapshot = true
unified_exec = true
memories = false
apps = false
plugins = false
undo = false
```

Use `/experimental` in the TUI for interactive toggles when available.

## Model Providers

Built-in providers are selected by `model_provider`. Custom OpenAI-compatible
providers can be added under `[model_providers.<id>]`:

```toml
model_provider = "acme"
model = "acme-coder-large"

[model_providers.acme]
name = "Acme"
base_url = "https://api.acme.example/v1"
env_key = "ACME_API_KEY"
wire_api = "responses"
```

Provider credentials should usually come from environment variables or the
credential store rather than inline tokens.

### app.nz

[app.nz](https://app.nz/docs) is a hosted OpenAI-compatible gateway. Add it as
a custom chat-completions provider and point `model` at its auto-router
(`app/auto`) or any `provider/model`:

```toml
model_provider = "appnz"
model = "app/auto"

[model_providers.appnz]
name = "app.nz"
base_url = "https://app.nz/v1"
env_key = "APPNZ_API_KEY"
wire_api = "chat"
```

Set `APPNZ_API_KEY` to your `app_live_...` key. Routing variants such as
`app/auto-code`, `app/auto-reasoning`, `app/auto-fast`, `app/auto-cheap`, and
`app/auto-vision` are also valid `model` values.

## Harness

Open Interpreter adds a `harness` setting for compatibility modes that shape
the agent surface like another coding harness while still running through the
native Open Interpreter runtime.

```toml
harness = "kimi-cli"
harness_guidance = true
```

Supported values are implementation-dependent, but the current codebase
includes native, Claude Code, DeepSeek TUI, Kimi CLI, Qwen Code, SWE-agent, and
minimal harness modes. Use this when you intentionally need a harness-shaped
prompt/tool surface. Leave it unset for normal Open Interpreter behavior.

`harness_guidance` lets Open Interpreter include a small reliability guidance
block where that harness mode allows it. Set it to `false` if you need stricter
harness behavior.

## MCP Servers

MCP servers are configured under `[mcp_servers]`:

```toml
[mcp_servers.docs]
command = "npx"
args = ["-y", "@acme/docs-mcp"]
env = { ACME_TOKEN = "env:ACME_TOKEN" }
default_tools_approval_mode = "prompt"
```

Streamable HTTP servers use `url`:

```toml
[mcp_servers.search]
url = "https://mcp.example.com"
bearer_token_env_var = "MCP_TOKEN"
```

See [MCP](/docs/mcp) for transport, OAuth, and per-tool approval details.

## Shell Environment

Use `shell_environment_policy` to control what environment variables are passed
to spawned commands:

```toml
[shell_environment_policy]
inherit = "all"
ignore_default_excludes = false
exclude = ["AWS_SECRET_ACCESS_KEY", "DATABASE_URL"]
set = { CI = "1" }
```

## History and Memory

Session history is stored locally. You can disable transcript persistence:

```toml
[history]
persistence = "none"
```

Memories are a separate experimental feature:

```toml
[features]
memories = true

[memories]
use_memories = true
generate_memories = true
```

See [Memories](/docs/memories).

## Config Schema

The source tree includes a generated JSON Schema at:

```text
codex-rs/core/config.schema.json
```

Use it for editor completion or CI validation when maintaining shared config.
