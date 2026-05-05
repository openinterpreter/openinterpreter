---
title: Slash commands
description: Quick actions you can run from the composer.
---

Type `/` in the composer to open the slash command picker. Start typing
to filter, then press `Enter` to run.

Some commands take inline arguments, like `/review fix the auth flow`.

## Models and personality

| Command         | What it does                                                  |
| --------------- | ------------------------------------------------------------- |
| `/model`        | Choose provider, model, and reasoning effort                  |
| `/fast`         | Toggle Fast mode on supported models                          |
| `/personality`  | Choose a communication style                                  |
| `/realtime`     | Toggle realtime voice mode (experimental)                     |
| `/settings`     | Configure realtime microphone and speaker                     |

## Permissions and sandbox

| Command                      | What it does                                                |
| ---------------------------- | ----------------------------------------------------------- |
| `/permissions`               | Pick what runs without asking                               |
| `/approvals`                 | Same as `/permissions`                                      |
| `/setup-default-sandbox`     | Set up an elevated agent sandbox                            |
| `/sandbox-add-read-dir <path>` | Grant the sandbox read access to a directory              |

## Conversation

| Command    | What it does                                                  |
| ---------- | ------------------------------------------------------------- |
| `/new`     | Start a fresh conversation in the same tab                    |
| `/resume`  | Open the resume picker                                        |
| `/fork`    | Fork the current chat into a new thread                       |
| `/rename`  | Rename the current thread                                     |
| `/clear`   | Clear the terminal and start a new chat                       |
| `/compact` | Summarize older turns to free context                         |
| `/side`    | Start a side conversation in an ephemeral fork                |
| `/agent`   | Switch the active agent thread                                |

## Files and changes

| Command      | What it does                                            |
| ------------ | ------------------------------------------------------- |
| `/init`      | Create an `AGENTS.md` file with project instructions    |
| `/diff`      | Show the working-tree diff (including untracked files)  |
| `/mention`   | Pin a file to the conversation                          |
| `/copy`      | Copy the latest output to your clipboard                |
| `/review`    | Review your current changes for issues                  |

## Modes

| Command        | What it does                                          |
| -------------- | ----------------------------------------------------- |
| `/plan`        | Switch to Plan mode for read-only thinking            |
| `/goal`        | Set or view the goal for a long-running task          |
| `/collab`      | Change collaboration mode (experimental)              |

## Tools and integrations

| Command       | What it does                                               |
| ------------- | ---------------------------------------------------------- |
| `/skills`     | Browse and toggle skills                                   |
| `/mcp`        | List configured MCP tools (`/mcp verbose` for details)     |
| `/apps`       | Manage app connectors                                      |
| `/plugins`    | Browse plugins                                             |
| `/memories`   | Configure memory use and generation                        |

## Background work

| Command  | What it does                       |
| -------- | ---------------------------------- |
| `/ps`    | List background terminals          |
| `/stop`  | Stop all background terminals      |

## Session info

| Command         | What it does                                              |
| --------------- | --------------------------------------------------------- |
| `/status`       | Show session configuration and token usage                |
| `/debug-config` | Print configuration layers and requirement sources        |
| `/rollout`      | Print the rollout file path                               |
| `/title`        | Configure terminal title fields                           |
| `/statusline`   | Configure status line fields                              |
| `/theme`        | Choose a syntax highlighting theme                        |

## App lifecycle

| Command       | What it does                  |
| ------------- | ----------------------------- |
| `/update`     | Manage Open Interpreter updates |
| `/feedback`   | Send logs to maintainers      |
| `/experimental` | Toggle experimental features |
| `/logout`     | Sign out                      |
| `/exit`       | Exit the CLI                  |
| `/quit`       | Same as `/exit`               |

<Tip>
The picker only shows commands that are valid right now. Commands like
`/model` are hidden while a task is running, and command availability
adapts to your active mode.
</Tip>
