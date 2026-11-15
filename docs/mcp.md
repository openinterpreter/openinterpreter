---
title: MCP
description: Connect Model Context Protocol servers for external tools and data.
---

Model Context Protocol (MCP) lets Open Interpreter call external tools exposed
by local or remote servers. MCP is useful for issue trackers, private docs,
databases, internal CLIs, and other capabilities that should be explicit rather
than improvised by shell commands.

## Add a Stdio Server

Configure a server in `~/.openinterpreter/config.toml`:

```toml
[mcp_servers.linear]
command = "npx"
args = ["-y", "@linear/mcp-server"]
env = { LINEAR_API_KEY = "env:LINEAR_API_KEY" }
```

Or use the CLI:

```bash
interpreter mcp add linear -- npx -y @linear/mcp-server
```

## Add an HTTP Server

```toml
[mcp_servers.docs]
url = "https://mcp.example.com"
bearer_token_env_var = "DOCS_MCP_TOKEN"
```

CLI form:

```bash
interpreter mcp add docs --url https://mcp.example.com \
  --bearer-token-env-var DOCS_MCP_TOKEN
```

## Manage Servers

```bash
interpreter mcp list
interpreter mcp get docs
interpreter mcp remove docs
interpreter mcp login docs
interpreter mcp logout docs
```

OAuth login applies to streamable HTTP servers that support OAuth.

Inside the TUI, use `/mcp` to see loaded servers and `/mcp verbose` for tool
details.

## Approval Modes

Set a server default:

```toml
[mcp_servers.docs]
command = "docs-mcp"
default_tools_approval_mode = "prompt"
```

Override one tool:

```toml
[mcp_servers.docs.tools.search]
approval_mode = "approve"
```

Common modes:

| Mode      | Behavior                      |
| --------- | ----------------------------- |
| `prompt`  | Ask before the tool runs.     |
| `approve` | Allow the tool automatically. |
| `auto`    | Let the active policy decide. |

## Tool Filtering

Expose only selected tools:

```toml
[mcp_servers.docs]
command = "docs-mcp"
enabled_tools = ["search", "read"]
disabled_tools = ["delete"]
```

`disabled_tools` is applied after `enabled_tools`.

## Timeouts and Startup

```toml
[mcp_servers.docs]
command = "docs-mcp"
startup_timeout_sec = 10
tool_timeout_sec = 60
required = true
enabled = true
```

If `required = true`, startup or resume fails when that server cannot initialize.

## Environment

For stdio servers:

```toml
[mcp_servers.local]
command = "my-mcp"
args = ["--stdio"]
cwd = "/Users/me/project"
env = { TOKEN = "env:MY_TOKEN" }
env_vars = ["PATH", "HOME"]
```

For HTTP servers:

```toml
[mcp_servers.remote]
url = "https://mcp.example.com"
http_headers = { "X-Client" = "open-interpreter" }
env_http_headers = { "Authorization" = "MCP_AUTH_HEADER" }
```

## Security Notes

Treat MCP tools like any other tool that can read, write, or call external
systems. Keep destructive tools on `prompt`, avoid storing secrets inline, and
review server configuration before enabling it for a project.
