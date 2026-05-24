---
title: Interactive mode
description: How the terminal UI works during a normal session.
---

Run `interpreter` in a project directory and you land in the terminal UI.
Type a request, watch the agent plan, edit files, and run commands.

```bash
cd my-project
interpreter
```

You can also start with a prompt right away:

```bash
interpreter "what does the auth middleware do?"
```

## The composer

The bottom of the screen is the composer. Type your message and press
`Enter` to send.

| Action                        | Keys              |
| ----------------------------- | ----------------- |
| Send message                  | `Enter`           |
| New line                      | `Shift+Enter`     |
| Slash command picker          | `/`               |
| Mention a file                | `@`               |
| Cancel current task           | `Esc`             |
| Quit                          | `Ctrl+C` twice    |

## Approving actions

When the agent wants to run a command that needs approval, it pauses
and shows you exactly what it plans to do.

| Key   | What it does                                       |
| ----- | -------------------------------------------------- |
| `y`   | Approve once                                       |
| `a`   | Approve and don't ask again for that command this session |
| `n`   | Deny                                               |
| `Esc` | Deny and tell the agent what to do differently     |

Approvals you remember last for the current session only. Quit and the
agent asks again next time.

To change how often the agent asks, run `/permissions` or read the
[sandbox guide](/docs/sandbox).

## Picking a model

Press `/model` to open the provider and model picker. The currently
active model shows in the footer.

For tasks that need fast iteration, try `/fast`. It uses the fastest
inference path the provider supports.

## Plan mode

`/plan` switches the agent into a read-only mode that thinks through a
problem before touching anything. Useful when the change is large enough
that you want to review the approach first.

When the plan looks right, leave plan mode and the agent executes it.

## Mentioning files

Type `@` to open a fuzzy file picker. Selected files are pinned to the
conversation as context. The same works with `/mention` if you prefer
typing the command.

## Showing diffs

`/diff` prints the working-tree diff inline, including untracked files,
without leaving the session.

## Reviewing changes

`/review` asks the agent to review your current changes for bugs and
regressions. It does not edit anything.

## Background terminals

The agent can spawn long-running terminals (a dev server, a watcher) and
keep working while they run.

| Command | What it does                          |
| ------- | -------------------------------------- |
| `/ps`   | List background terminals              |
| `/stop` | Stop all background terminals          |

## Switching between tabs

Open Interpreter is designed for many tabs without each one acting like a
separate runtime. Configuration and a shared local backend keep memory
usage flat as you add tabs.

The TUI also uses low-memory client behavior by default: completed transcript
cells are not retained in every live client's in-memory transcript list, and
large active tool outputs are capped in the live client. Set
`INTERPRETER_TUI_LOW_MEMORY=0` before launch to disable this behavior for
debugging.

## Quitting

| Command       | What it does                                          |
| ------------- | ----------------------------------------------------- |
| `/exit`       | Close the session                                     |
| `Ctrl+C` x2   | Force quit (a one-second hint window prevents typos)  |
| `interpreter kill` | Stop the local daemon entirely                   |
