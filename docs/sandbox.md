---
title: Sandbox & approvals
description: Two layers that decide what the agent can run on its own.
---

Open Interpreter runs commands and edits files on your behalf. Two
controls decide what is allowed and what asks first:

- **Sandbox mode** sets the technical boundary. Files and network access.
- **Approval mode** sets the human checkpoint. When to pause and ask.

They work together. The sandbox decides what is even possible. Approvals
decide which possible actions still need a yes from you.

## Sandbox modes

| Mode                 | What the agent can do                                                                       |
| -------------------- | ------------------------------------------------------------------------------------------- |
| `read-only`          | Read files and answer questions. No edits, no commands, no network. The default.            |
| `workspace-write`    | Edit files and run commands inside the current workspace. Network is off unless you opt in. |
| `danger-full-access` | No technical limits. Edit anywhere, hit the network freely.                                 |

Set the default in `config.toml`:

```toml
sandbox_mode = "workspace-write"
```

Override for a single run:

```bash
interpreter --sandbox read-only "audit the auth flow"
```

### Workspace-write extras

You can grant the sandbox extra read paths without leaving the session:

```
/sandbox-add-read-dir /Users/me/notes
```

Or up front in `config.toml`:

```toml
[sandbox]
extra_read_dirs = ["/Users/me/notes", "/var/log"]
```

## Approval modes

| Mode          | When the agent stops to ask                                                          |
| ------------- | ------------------------------------------------------------------------------------ |
| `untrusted`   | Safe reads run on their own. Anything that could change state asks first.            |
| `on-request`  | The agent runs whatever the sandbox allows. It asks before stepping outside it. The default. |
| `never`       | No prompts. The sandbox is the only guardrail.                                       |

Set it in `config.toml`:

```toml
approval_policy = "on-request"
```

Or change it mid-session:

```
/permissions
```

## Picking a combo

| Goal                         | Sandbox            | Approvals     |
| ---------------------------- | ------------------ | ------------- |
| Browse a new codebase safely | `read-only`        | `on-request`  |
| Day-to-day work              | `workspace-write`  | `on-request`  |
| Trusted automation           | `workspace-write`  | `never`       |
| Quick local hacking          | `danger-full-access` | `never`     |

If you are unsure, start with `workspace-write` and `on-request`. You
get fast iteration with a guardrail before anything reaches outside the
project.

<Warning>
`danger-full-access` removes the sandbox. The agent can change files
anywhere, run any command, and use the network freely. Pair it with
strong approvals or use it only on machines you can throw away.
</Warning>

## During a session

When the agent wants to run something that needs a yes:

| Key   | What it does                                           |
| ----- | ------------------------------------------------------ |
| `y`   | Approve once                                           |
| `a`   | Approve and don't ask again for that command this session |
| `n`   | Deny                                                   |
| `Esc` | Deny and tell the agent what to do differently         |

Approvals you remember last for the current session only. Quit and the
agent asks again next time.
