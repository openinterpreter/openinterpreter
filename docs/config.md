---
title: Configuration
description: Tune Open Interpreter from one TOML file.
---

Open Interpreter reads its configuration from `~/.openinterpreter/config.toml`.
You can also drop a `.openinterpreter/config.toml` into a project to layer
project-specific settings on top.

## Where settings come from

Settings layer in this order, with later layers winning:

<Steps>
  <Step title="Built-in defaults">
    Sensible values that ship with the binary.
  </Step>
  <Step title="System config">
    Admin-managed values, if present.
  </Step>
  <Step title="User config">
    `~/.openinterpreter/config.toml`. Your defaults.
  </Step>
  <Step title="Project config">
    `.openinterpreter/config.toml` inside the project root.
  </Step>
  <Step title="Profile">
    A named profile selected with `--profile`.
  </Step>
  <Step title="CLI flags">
    `-c key=value` overrides for a single run.
  </Step>
</Steps>

To see exactly where each value came from:

```
/debug-config
```

## The settings you reach for most

```toml
# Default model and provider
model = "gpt-5-codex"

# When to ask before running things: "untrusted" | "on-request" | "never"
approval_policy = "on-request"

# Sandbox: "read-only" | "workspace-write" | "danger-full-access"
sandbox_mode = "workspace-write"

# Reasoning effort: "low" | "medium" | "high" | "none"
model_reasoning_effort = "medium"

# Harness guidance: lets selected harnesses include Open Interpreter
# reliability and tool-use guidance. Set false for stricter harness emulation.
harness_guidance = true

# Communication style
personality = "concise"

# Where the agent writes logs
log_dir = "~/.openinterpreter/log"
```

## Profiles

Profiles are named groups of overrides for different contexts.

```toml
[profiles.fast]
model = "gpt-5-codex-mini"
model_reasoning_effort = "low"

[profiles.review]
model = "gpt-5-codex"
model_reasoning_effort = "high"
sandbox_mode = "read-only"
```

Use one with:

```bash
interpreter --profile review
```

## Override values from the CLI

`-c` takes a TOML expression and applies it to the active config:

```bash
interpreter -c model='"gpt-5-codex-mini"' -c approval_policy='"never"'
```

Useful for one-off runs and scripts.

## Harness guidance

When `harness_guidance` is enabled, a selected harness can add a small
Open Interpreter guidance block to the model instructions. This is intended
to make tool use more reliable while preserving the selected harness behavior.

It defaults to `true`. Disable it when you want stricter emulation:

```toml
harness_guidance = false
```

For a single run:

```bash
interpreter -c harness_guidance=false "fix the failing tests"
```

## MCP servers

Open Interpreter can connect to [Model Context Protocol](https://modelcontextprotocol.io)
servers. Define them under `[mcp_servers]`:

```toml
[mcp_servers.docs]
command = "docs-server"
```

By default, MCP tools run one at a time. To allow parallel calls for a
server whose tools are safe to run together:

```toml
[mcp_servers.docs]
command = "docs-server"
supports_parallel_tool_calls = true
```

<Warning>
Only enable parallel calls for tools that do not share state. Two tools
that write to the same file or row at the same time will race.
</Warning>

### MCP tool approvals

Set a default approval mode for every tool exposed by a server, and
override individual tools:

```toml
[mcp_servers.docs]
command = "docs-server"
default_tools_approval_mode = "approve"

[mcp_servers.docs.tools.search]
approval_mode = "prompt"
```

See the [MCP servers guide](/docs/mcp) for the full picture.

## Feature flags

Optional behavior lives behind `[features]`.

```toml
[features]
apps = true            # ChatGPT connector surface ($ mentions, /apps)
plugins = true         # Plugin bundles for skills, MCP, connectors
child_agents_md = true # Hierarchical AGENTS.md guidance
```

Apps and plugins stay off by default so the base config stays
provider-neutral.

## Notify hook

Open Interpreter can run a shell hook each time a turn finishes. Wire it
up under `[notify]`. The notification payload includes a `client` field
identifying the surface that started the turn (the TUI reports
`interpreter-tui`).

## Custom CA certificates

Behind a corporate proxy that intercepts TLS? Point Open Interpreter at
your bundle:

```bash
export CODEX_CA_CERTIFICATE=/etc/ssl/corp-bundle.pem
```

If `CODEX_CA_CERTIFICATE` is not set, Open Interpreter falls back to
`SSL_CERT_FILE`. If neither is set, it uses your system root certificates.

The PEM file may contain multiple certificates. OpenSSL `TRUSTED CERTIFICATE`
labels are tolerated, and well-formed `X509 CRL` sections are ignored.

## SQLite state

Sessions and memory are stored in SQLite under `sqlite_home`. Override it
with the config key or `CODEX_SQLITE_HOME`. By default,
`workspace-write` sessions store under a temp dir, while other sandboxes
use `~/.openinterpreter`.

## Plan mode defaults

`plan_mode_reasoning_effort` lets you override reasoning effort while in
Plan mode. Set it to `"none"` to explicitly disable reasoning when planning.
Leave it unset to inherit the Plan preset default (`medium`).

## Notices

The `[notice]` table stores "do not show again" flags for some UI prompts.
Delete entries here if you want a notice to appear again.

## JSON Schema

A generated JSON Schema for `config.toml` lives at
`codex-rs/core/config.schema.json` in the source tree. Useful for editor
autocomplete or validation in CI.

## Quitting hint

Pressing `Ctrl+C` once shows a one-second hint (`ctrl + c again to quit`).
The second press exits. This catches accidental quits without making
intentional exits slow.
