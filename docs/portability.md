---
title: Portability
description: How Open Interpreter uses shared agent standards and avoids product lock-in.
---

Open Interpreter's goal is to participate in a shared agent ecosystem, not
create a private island of instructions, skills, and workflows. When a
practical cross-tool standard exists, Open Interpreter should read it directly
and preserve it in a form other compatible tools can use.

This is a product direction and an engineering constraint. It is not a claim
that every kind of runtime state already has a universal format.

## Principles

- Prefer established, tool-neutral protocols and directory conventions over
  Open Interpreter-specific equivalents.
- Keep user-authored instructions, skills, and configuration readable and
  inspectable.
- Read shared content in place instead of requiring an import or private copy.
- Keep compatibility readers for legacy product paths during migrations.
- Make export and migration possible when no shared live format exists.
- Reserve product-specific storage for secrets, caches, indexes, logs, and
  runtime state that cannot yet be represented safely by a shared standard.

## Current Shared Surface

| Capability                    | Shared surface                 |
| ----------------------------- | ------------------------------ |
| Project instructions          | `AGENTS.md`                    |
| Project skills                | `.agents/skills/`              |
| Personal skills               | `~/.agents/skills/`            |
| Tool integrations             | Model Context Protocol (MCP)   |
| Editor and client integration | Agent Client Protocol (ACP)    |
| Programmatic execution        | Codex-compatible exec protocol |

These locations and protocols should work without converting the user's data
into an Open Interpreter-only representation.

## Product-Specific State

Open Interpreter currently keeps configuration, credentials, session history,
logs, caches, and daemon state under `~/.openinterpreter` or the operating
system credential store. Some of that state is inherently product-specific;
some may move to a shared standard when a safe and broadly adopted one exists.

The legacy `~/.openinterpreter/skills/` directory remains readable so existing
setups do not break. Shared `~/.agents/skills/` is the preferred home for new
personal skills.

## What This Means for Changes

Before adding a new product-owned file format or directory, first check whether
an established agent, editor, or operating-system standard can represent the
same data. If a product-specific format is still necessary:

1. Use plain, documented data where practical.
2. Keep the boundary narrow.
3. Provide a migration or export path for user-authored data.
4. Avoid making the product-specific copy the only usable source of truth.

The test for a portable feature is simple: a user should be able to understand
where their data lives, reuse the standardized parts with another compatible
tool, and leave Open Interpreter without losing user-authored work.
