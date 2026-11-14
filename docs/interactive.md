---
title: Interactive Mode
description: Work in the terminal UI with prompts, files, images, approvals, and slash commands.
---

Run `interpreter` in a project directory to start the terminal UI:

```bash
cd my-project
interpreter
```

You can also pass the first prompt on the command line:

```bash
interpreter "find the auth middleware and explain how it works"
```

## Composer

The composer is the prompt box at the bottom of the TUI.

| Action                                  | Keys or command           |
| --------------------------------------- | ------------------------- |
| Send message                            | `Enter`                   |
| Add a newline                           | `Shift+Enter`             |
| Open slash commands                     | `/`                       |
| Mention files                           | `@` or `/mention`         |
| Edit prompt in `$VISUAL` or `$EDITOR`   | `Ctrl+G`                  |
| Search prompt history                   | `Ctrl+R`                  |
| Queue a follow-up while work is running | `Tab`                     |
| Cancel or back out                      | `Esc`                     |
| Quit                                    | `/exit` or `Ctrl+C` twice |

## Files and Images

Use `@` to fuzzy-search files and add them as context. You can also attach
images to the first prompt:

```bash
interpreter -i screenshot.png "explain what is wrong in this UI"
interpreter -i before.png,after.png "compare these states"
```

## Approvals

When a command or tool needs approval, the TUI shows the request before it runs.
The default posture is designed for day-to-day work in a trusted repository:
workspace access is allowed, and actions outside the active policy ask first.

Change the active policy with:

```text
/permissions
```

For details, see [Sandbox & approvals](/docs/sandbox) and
[Permissions](/docs/permissions).

## Models and Providers

Use `/model` to pick the provider, model, and reasoning effort. Open
Interpreter supports OpenAI, Anthropic, local providers, and compatible custom
providers from the generated model catalog.

Common one-off overrides:

```bash
interpreter -m gpt-5.1-codex "review this module"
interpreter --oss "try this with my local model"
```

## Review and Planning

Use `/plan` when you want the agent to inspect and propose before it edits. Use
`/review` when you want a code-review pass over current changes.

```text
/plan
/review
```

Review mode is read-focused. It reports bugs, regressions, missing tests, and
risky behavior before summaries.

## Background Work

Long-running commands can stay alive in background terminals while the agent
continues.

| Command | Purpose                   |
| ------- | ------------------------- |
| `/ps`   | List background terminals |
| `/stop` | Stop background terminals |

## Session Controls

| Command    | Purpose                                            |
| ---------- | -------------------------------------------------- |
| `/new`     | Start a fresh conversation                         |
| `/resume`  | Pick an older session                              |
| `/fork`    | Branch the current session                         |
| `/compact` | Summarize older context                            |
| `/clear`   | Clear the screen                                   |
| `/copy`    | Copy the latest assistant output                   |
| `/theme`   | Change syntax highlighting theme                   |
| `/status`  | Inspect model, sandbox, approvals, and token state |

Open Interpreter keeps session state locally under `~/.openinterpreter/`.
