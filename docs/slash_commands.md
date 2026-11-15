---
title: Slash Commands
description: Commands available from the interactive composer.
---

Type `/` in the composer to open the command picker. The list adapts to the
current mode and active feature flags.

## Models and Behavior

| Command        | Purpose                                                                 |
| -------------- | ----------------------------------------------------------------------- |
| `/model`       | Choose provider, model, and reasoning effort.                           |
| `/fast`        | Use the fastest supported service tier where available.                 |
| `/personality` | Select the communication style.                                         |
| `/theme`       | Change syntax highlighting theme.                                       |
| `/status`      | Show model, provider, sandbox, approvals, token use, and session state. |

## Permissions and Sandboxing

| Command                        | Purpose                                      |
| ------------------------------ | -------------------------------------------- |
| `/permissions`                 | Choose the active permission posture.        |
| `/approvals`                   | Alias for permissions in compatible builds.  |
| `/sandbox-add-read-dir <path>` | Grant read access to an extra directory.     |
| `/setup-default-sandbox`       | Configure a default sandbox where supported. |

## Conversation and Sessions

| Command          | Purpose                                    |
| ---------------- | ------------------------------------------ |
| `/new`           | Start a new conversation.                  |
| `/resume`        | Resume an older session.                   |
| `/fork`          | Branch the current session into a new one. |
| `/side`          | Start a side conversation in a fork.       |
| `/agent`         | Switch or manage subagent threads.         |
| `/clear`         | Clear the visible transcript.              |
| `/compact`       | Summarize old context to make room.        |
| `/rename`        | Rename the current thread.                 |
| `/exit`, `/quit` | Leave the TUI.                             |

## Files and Code

| Command    | Purpose                                          |
| ---------- | ------------------------------------------------ |
| `/init`    | Create an `AGENTS.md` with project guidance.     |
| `/mention` | Add files to the conversation.                   |
| `/diff`    | Show the current working-tree diff.              |
| `/review`  | Review current changes for bugs and regressions. |
| `/copy`    | Copy the latest assistant response.              |

## Integrations

| Command     | Purpose                                     |
| ----------- | ------------------------------------------- |
| `/mcp`      | Show configured MCP servers and tools.      |
| `/skills`   | Inspect available skills.                   |
| `/plugins`  | Browse plugin support when enabled.         |
| `/apps`     | Manage connector/app tools when enabled.    |
| `/memories` | Inspect or configure memories when enabled. |
| `/hooks`    | Review and trust lifecycle hooks.           |

## Background Work

| Command | Purpose                         |
| ------- | ------------------------------- |
| `/ps`   | List background terminal tasks. |
| `/stop` | Stop background tasks.          |

## Diagnostics and Lifecycle

| Command         | Purpose                                     |
| --------------- | ------------------------------------------- |
| `/debug-config` | Print resolved configuration and sources.   |
| `/rollout`      | Show the local transcript path.             |
| `/title`        | Configure terminal title behavior.          |
| `/statusline`   | Configure status line behavior.             |
| `/experimental` | Toggle experimental features where exposed. |
| `/update`       | Manage standalone CLI updates.              |
| `/logout`       | Remove saved credentials.                   |
| `/feedback`     | Send feedback and optional logs.            |
