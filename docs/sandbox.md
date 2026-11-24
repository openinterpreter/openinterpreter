---
title: Sandbox & Approvals
description: Control what local commands can do and when Open Interpreter asks first.
---

Open Interpreter has two separate safety controls:

- Sandbox mode controls the technical boundary for local command execution.
- Approval policy controls when the agent pauses and asks you.

Use `/permissions` in the TUI to inspect or change the active posture.

## Sandbox Modes

| Mode                 | Behavior                                                                             |
| -------------------- | ------------------------------------------------------------------------------------ |
| `read-only`          | Commands can inspect allowed files but cannot write.                                 |
| `workspace-write`    | Commands can write inside the active workspace roots. Network is off unless enabled. |
| `danger-full-access` | No local sandbox boundary. Use only in an environment you intentionally trust.       |

Set a default:

```toml
sandbox_mode = "workspace-write"
```

Override once:

```bash
interpreter --sandbox read-only "audit the auth flow"
```

## Approval Policies

| Policy       | Behavior                                          |
| ------------ | ------------------------------------------------- |
| `untrusted`  | Ask before actions that could change state.       |
| `on-request` | Run inside the sandbox and ask before escalation. |
| `never`      | Do not ask. The sandbox is the only guardrail.    |

```toml
approval_policy = "on-request"
```

`--yolo` and `--dangerously-bypass-approvals-and-sandbox` remove both approval
prompts and sandboxing. Use them only inside an external sandbox such as a
throwaway VM or isolated container.

## Workspace Write

Grant extra writable roots for a session:

```bash
interpreter --add-dir ../shared-lib
```

Enable network for the older workspace-write sandbox:

```toml
[sandbox_workspace_write]
network_access = true
```

For precise network allowlists, use [Permissions](/docs/permissions).

## Protected Paths

Even inside writable roots, sensitive control directories such as `.git/` and
agent configuration directories should be treated as protected. If the agent
needs to change them, review the request closely.

## Operating System Enforcement

Open Interpreter uses the same local sandbox architecture as the Codex CLI
surface:

| Platform    | Enforcement model                                                     |
| ----------- | --------------------------------------------------------------------- |
| macOS       | Seatbelt profiles.                                                    |
| Linux / WSL | Bubblewrap, seccomp, and related kernel sandboxing where available.   |
| Windows     | Native Windows sandboxing where configured; WSL uses the Linux model. |

When a requested policy cannot be enforced, Open Interpreter should fail closed
rather than silently running unsandboxed.

## Recommended Defaults

| Situation                          | Suggested settings                                             |
| ---------------------------------- | -------------------------------------------------------------- |
| Inspecting unfamiliar code         | `sandbox_mode = "read-only"`, `approval_policy = "on-request"` |
| Day-to-day trusted repo work       | `workspace-write` plus `on-request`                            |
| CI in an isolated runner           | `workspace-write` plus `never`                                 |
| Disposable full-access environment | `danger-full-access` plus `never`                              |

If you are unsure, start with workspace-write and on-request.
