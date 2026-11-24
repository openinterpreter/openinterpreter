---
title: Hooks
description: Run trusted scripts at specific points in the agent lifecycle.
---

Hooks let you run deterministic commands around Open Interpreter events. Use
them for policy checks, logging, prompt scanning, custom context injection, or
post-run validation.

Hooks are enabled by default:

```toml
[features]
hooks = true
```

Disable them only when you intentionally want no lifecycle scripts:

```toml
[features]
hooks = false
```

## Where Hooks Live

Open Interpreter discovers hooks next to active config layers:

| Location                         | Scope                        |
| -------------------------------- | ---------------------------- |
| `~/.openinterpreter/hooks.json`  | User                         |
| `~/.openinterpreter/config.toml` | User inline hooks            |
| `.openinterpreter/hooks.json`    | Trusted project              |
| `.openinterpreter/config.toml`   | Trusted project inline hooks |
| Enabled plugins                  | Plugin-bundled hooks         |

If multiple sources match, they all run. Higher-precedence config does not
replace hooks from lower-precedence layers.

## Trust

Non-managed command hooks must be reviewed and trusted before they run. Open
Interpreter records trust against the exact hook definition, so changed hooks
need review again.

Use:

```text
/hooks
```

Automation that already validates hooks externally can pass
`--dangerously-bypass-hook-trust`, but that should be rare.

## Events

| Event               | When it runs                                       |
| ------------------- | -------------------------------------------------- |
| `SessionStart`      | A session starts, resumes, clears, or compacts.    |
| `UserPromptSubmit`  | A user prompt is about to be sent.                 |
| `PreToolUse`        | Before supported shell, patch, or MCP tools run.   |
| `PermissionRequest` | Before an approval prompt is shown.                |
| `PostToolUse`       | After supported shell, patch, or MCP tools finish. |
| `PreCompact`        | Before context compaction.                         |
| `PostCompact`       | After context compaction.                          |
| `SubagentStart`     | A subagent starts.                                 |
| `SubagentStop`      | A subagent stops.                                  |
| `Stop`              | A turn is about to finish.                         |

## JSON Form

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "^Bash$",
        "hooks": [
          {
            "type": "command",
            "command": "python3 .openinterpreter/hooks/pre_tool_use.py",
            "timeout": 30,
            "statusMessage": "Checking command"
          }
        ]
      }
    ]
  }
}
```

## TOML Form

```toml
[[hooks.PreToolUse]]
matcher = "^Bash$"

[[hooks.PreToolUse.hooks]]
type = "command"
command = "python3 .openinterpreter/hooks/pre_tool_use.py"
timeout = 30
statusMessage = "Checking command"
```

## Matchers

Matchers are regular expressions. Supported matcher targets include tool names
for tool events, `startup|resume|clear|compact` for `SessionStart`, and
`manual|auto` for compaction events.

Examples:

```text
Bash
^apply_patch$
Edit|Write
mcp__filesystem__read_file
startup|resume
manual|auto
```

## Hook Input and Output

Command hooks receive one JSON object on stdin with fields such as
`session_id`, `cwd`, `hook_event_name`, `model`, and event-specific fields.

Some events can add model-visible context. Some can block or deny a tool call.
For example, a `PreToolUse` hook can deny a command:

```json
{
  "hookSpecificOutput": {
    "hookEventName": "PreToolUse",
    "permissionDecision": "deny",
    "permissionDecisionReason": "Blocked by repository policy."
  }
}
```

Hooks are guardrails, not a substitute for sandboxing and approvals.
