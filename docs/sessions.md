---
title: Sessions
description: Resume earlier work, fork a conversation, and manage history.
---

Every Open Interpreter conversation is recorded locally so you can pick up
later. Sessions live under `~/.openinterpreter/` and stay on your machine.

## Resume the last session

```bash
interpreter resume --last
```

You land back in the same conversation with the same files in context.

## Pick from a list

```bash
interpreter resume
```

A picker shows recent sessions in the current directory. Add `--all` to
see sessions from anywhere on your machine.

## Fork a session

Forking branches a conversation into a new thread. The original stays
intact. Useful when you want to try a different approach without losing
the existing one.

```bash
interpreter fork --last
```

Or pick from the list:

```bash
interpreter fork
```

## Inside a session

| Command   | What it does                                 |
| --------- | -------------------------------------------- |
| `/new`    | Start a fresh conversation in the same tab   |
| `/fork`   | Fork the current conversation                |
| `/resume` | Open the resume picker                       |
| `/rename` | Rename the current thread                    |
| `/clear`  | Clear the screen and start a fresh chat      |
| `/compact`| Summarize old turns to free up context       |

## Stop the daemon

Open Interpreter runs a small local daemon so multiple tabs share state
efficiently. To stop it:

```bash
interpreter kill
```

Add `--force` if you want it to exit immediately without cleanup.
