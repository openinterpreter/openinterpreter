---
title: Subagents
description: Use specialized helper agents for investigation, review, and parallel work.
---

Subagents are separate agent threads that can work alongside the main session.
They are useful for isolated investigation, broad code search, review passes,
or parallel exploration.

The multi-agent feature is enabled by default in current builds:

```toml
[features]
multi_agent = true
```

## In the TUI

Use:

```text
/agent
```

The main agent may also spawn subagents explicitly when the task benefits from
parallel work.

## Built-In Roles

Common built-in roles include:

| Role       | Purpose                                 |
| ---------- | --------------------------------------- |
| `default`  | General-purpose helper.                 |
| `worker`   | Focused execution or investigation.     |
| `explorer` | Read-heavy discovery and summarization. |

Available roles can vary by build and config.

## Settings

```toml
[agents]
max_threads = 6
max_depth = 1
job_max_runtime_seconds = 1800
```

| Key                       | Meaning                                    |
| ------------------------- | ------------------------------------------ |
| `max_threads`             | Maximum concurrent agent threads.          |
| `max_depth`               | How deeply agents can spawn other agents.  |
| `job_max_runtime_seconds` | Default timeout for CSV/batch worker jobs. |

## Custom Agent

Define a role in config:

```toml
[agents.explorer]
description = "Inspect code and report findings without editing."
developer_instructions = "Stay read-only. Prefer rg and direct file references."
model = "gpt-5.1-codex"
model_reasoning_effort = "medium"
sandbox_mode = "read-only"
```

Useful optional fields include `nickname_candidates`, `mcp_servers`, and
skill configuration.

## Permissions

Subagents inherit the active sandbox and approval posture unless their role
configuration narrows it. Approval prompts can surface even when a subagent is
not the currently visible thread.
