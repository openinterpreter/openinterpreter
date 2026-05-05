---
title: MCP servers
description: Give Open Interpreter access to external tools and data through Model Context Protocol.
---

[MCP](https://modelcontextprotocol.io) is an open protocol for exposing
tools and data to AI assistants. Open Interpreter can connect to any MCP
server and use its tools alongside the built-in ones.

Common uses:

- Pull issues from a tracker
- Query a private knowledge base
- Run domain-specific commands the agent should not invent

## Configure a server

Add it to `~/.openinterpreter/config.toml` under `[mcp_servers.<name>]`:

```toml
[mcp_servers.linear]
command = "npx"
args = ["-y", "@linear/mcp-server"]
env = { LINEAR_API_KEY = "lin_api_..." }
```

The server is launched on demand and shut down when you exit the session.

## Browse what is loaded

Inside a session:

```
/mcp
```

Add `verbose` to see every tool a server exposes:

```
/mcp verbose
```

## Approval modes

By default, every MCP tool call asks for approval. Loosen it for trusted
servers:

```toml
[mcp_servers.docs]
command = "docs-server"
default_tools_approval_mode = "approve"
```

Override individual tools:

```toml
[mcp_servers.docs.tools.delete_doc]
approval_mode = "prompt"
```

| Mode      | Behavior                                  |
| --------- | ----------------------------------------- |
| `prompt`  | Ask every time                            |
| `approve` | Run automatically                         |
| `deny`    | Refuse the tool call                      |

## Parallel tool calls

MCP tools run one at a time by default. If a server's tools are safe to
run together, opt in:

```toml
[mcp_servers.search]
command = "search-mcp"
supports_parallel_tool_calls = true
```

<Warning>
Only enable this for tools that do not share state. Two tools writing to
the same file or row at once will race.
</Warning>

## Remote servers

Open Interpreter can also connect to remote MCP servers over a URL.

```toml
[mcp_servers.acme]
url = "https://mcp.acme.com"
auth_token_env = "ACME_TOKEN"
```

The token is read from the named environment variable so secrets stay
out of the file.

## Troubleshooting

| Symptom                              | Try this                                                |
| ------------------------------------ | ------------------------------------------------------- |
| Tool does not show up in `/mcp`      | Run `/debug-config` and confirm the server is loaded    |
| Server crashes on startup            | Run the `command` directly in a shell to see the error  |
| Calls fail silently                  | Tail `~/.openinterpreter/log/interpreter-tui.log`       |
