---
title: Migrate
description: Bring Codex or other compatible agent setup into Open Interpreter.
---

Open Interpreter can reuse much of the Codex-style local setup because the CLI
surface and configuration model are closely related.

The long-term goal is that standards-based agent setup does not need a
migration at all: Open Interpreter should read shared files and directories in
place whenever a practical cross-tool standard exists.

## What Usually Migrates

| Source item             | Open Interpreter destination                                       |
| ----------------------- | ------------------------------------------------------------------ |
| Instructions            | `AGENTS.md`                                                        |
| Config                  | `~/.openinterpreter/config.toml` or `.openinterpreter/config.toml` |
| Skills                  | `.agents/skills/` or `~/.agents/skills/`                           |
| MCP config              | `[mcp_servers]`                                                    |
| Hooks                   | `hooks.json` or inline `[hooks]`                                   |
| Slash-command workflows | Skills or project instructions                                     |
| Subagents               | `[agents]` config                                                  |
| Recent sessions         | Local session history where supported                              |

Skills already stored under `.agents/skills/` or `~/.agents/skills/` do not
need to be copied. Open Interpreter reads those shared locations directly.

## Review After Import

Review migrated setup before relying on it:

- MCP servers with custom auth, headers, or transports
- Hooks that run local commands
- Skill scripts and references
- Agent permissions and tool restrictions
- Prompt templates that depend on shell interpolation or path placeholders

## Codex Home

Open Interpreter uses `~/.openinterpreter/` for product-specific configuration
and runtime state that does not yet have a practical shared home. If you
previously used Codex, inspect both homes during migration:

```text
~/.codex/
~/.openinterpreter/
```

Do not copy secrets blindly. Prefer re-authenticating or using environment
variables. Keep portable, user-authored content such as skills in the shared
locations above rather than either product home.
