---
title: Customization
description: Teach Open Interpreter your project, tools, workflows, and preferences.
---

Open Interpreter customization is layered. Use the smallest durable mechanism
that fits the job.

| Mechanism       | Best for                                                         |
| --------------- | ---------------------------------------------------------------- |
| `AGENTS.md`     | Project rules, commands, architecture, and conventions.          |
| Config profiles | Model, provider, sandbox, approval, and feature defaults.        |
| Skills          | Repeatable workflows with instructions, scripts, and references. |
| MCP             | External tools and data sources.                                 |
| Hooks           | Deterministic policy, logging, and validation scripts.           |
| Subagents       | Specialized helper agents for investigation or parallel work.    |
| Memories        | Personal cross-session preferences when enabled.                 |

## Recommended Pattern

1. Put stable repo instructions in `AGENTS.md`.
2. Put personal defaults in `~/.openinterpreter/config.toml`.
3. Put project-specific config in `.openinterpreter/config.toml` only for
   trusted repositories.
4. Use skills for workflows you repeat.
5. Use MCP for tools the agent should call directly.
6. Use hooks for deterministic checks that should run every time.

## Keep Instructions Small

Long instruction files make every session heavier. Prefer short rules and links
to concrete files. Move large procedures into skills so they load only when
needed.

## Review Customization

Customization can change what tools run and what context the model sees. Review
repo-local config, skills, MCP servers, and hooks before trusting a new project.
