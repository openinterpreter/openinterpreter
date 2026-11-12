---
title: CLI Reference
description: Main Open Interpreter commands and flags.
---

The public command is `interpreter`. With no subcommand it starts the
app-server-backed terminal UI.

## Global Flags

| Flag                              | Purpose                                                         |
| --------------------------------- | --------------------------------------------------------------- |
| `PROMPT`                          | Optional initial prompt.                                        |
| `--image, -i <path[,path...]>`    | Attach image files to the initial prompt.                       |
| `--model, -m <model>`             | Override the configured model.                                  |
| `--oss`                           | Use the configured local open source provider.                  |
| `--local-provider <provider>`     | Use `ollama` or `lmstudio` with `--oss`.                        |
| `--profile, -p <name>`            | Load a config profile.                                          |
| `--sandbox, -s <mode>`            | Select `read-only`, `workspace-write`, or `danger-full-access`. |
| `--ask-for-approval, -a <policy>` | Select `untrusted`, `on-request`, or `never`.                   |
| `--cd, -C <path>`                 | Start in another working directory.                             |
| `--add-dir <path>`                | Add a writable workspace root.                                  |
| `--search`                        | Request live web search.                                        |
| `--enable <feature>`              | Enable a feature for this run.                                  |
| `--disable <feature>`             | Disable a feature for this run.                                 |
| `--config, -c key=value`          | Override config for this run.                                   |
| `--remote <ws-url>`               | Connect the TUI to a remote app-server endpoint.                |
| `--remote-auth-token-env <var>`   | Read the remote bearer token from an environment variable.      |
| `--no-alt-screen`                 | Disable alternate-screen TUI mode.                              |
| `--yolo`                          | Bypass approvals and sandboxing. Dangerous.                     |

## Commands

| Command                         | Purpose                                                                       |
| ------------------------------- | ----------------------------------------------------------------------------- |
| `interpreter`                   | Start the TUI.                                                                |
| `interpreter resume`            | Resume an interactive session.                                                |
| `interpreter fork`              | Fork an interactive session.                                                  |
| `interpreter exec`              | Run non-interactively.                                                        |
| `interpreter acp`               | Run as an Agent Client Protocol agent over stdio.                             |
| `interpreter app-server daemon` | Manage the optional shared [daemon](/docs/daemon) (`start`/`stop`/`restart`). |
| `interpreter mcp`               | Manage MCP server config.                                                     |
| `interpreter mcp-server`        | Expose Open Interpreter as an MCP server over stdio.                          |
| `interpreter update`            | Run the configured installer for the latest release.                          |

## Resume

```bash
interpreter resume
interpreter resume --last
interpreter resume --all
interpreter resume <SESSION_ID>
```

## Fork

```bash
interpreter fork
interpreter fork --last
interpreter fork --all
interpreter fork <SESSION_ID>
```

## Exec

```bash
interpreter exec "fix the failing test"
interpreter exec --json "summarize this repo"
interpreter exec resume --last "continue"
interpreter exec review --uncommitted
```

See [Non-interactive mode](/docs/exec).

## MCP

```bash
interpreter mcp list
interpreter mcp get <name>
interpreter mcp add <name> -- <command...>
interpreter mcp add <name> --url https://mcp.example.com
interpreter mcp remove <name>
interpreter mcp login <name>
interpreter mcp logout <name>
```

See [MCP](/docs/mcp).

## MCP Server

```bash
interpreter mcp-server
```

See [MCP server](/docs/mcp-server).

## ACP

```bash
interpreter acp
```

See [Agent Client Protocol](/docs/acp).

## Update

```bash
interpreter update
```

Automatic checks during interactive startup are controlled by
`check_for_update_on_startup`.

## Notes on Codex Compatibility

Open Interpreter preserves the Codex CLI interaction model for local coding:
interactive TUI, `exec`, sessions, slash commands, MCP, sandboxing, approvals,
skills, hooks, and subagents. OpenAI-hosted Codex Cloud commands are not part of
the Open Interpreter public surface unless a future Open Interpreter release
adds an equivalent.
