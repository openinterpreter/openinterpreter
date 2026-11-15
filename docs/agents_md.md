---
title: AGENTS.md
description: Durable project instructions that Open Interpreter reads automatically.
---

`AGENTS.md` is the project instruction file. Put stable guidance there instead
of repeating it in every prompt.

Use it for:

- Build, test, lint, and format commands
- Project architecture notes
- Code style and API conventions
- Files or directories that need care
- Release, migration, or review expectations

## Create One

Inside the TUI:

```text
/init
```

Open Interpreter inspects the repository and drafts a starting `AGENTS.md`.
Edit it down to the rules that should survive across sessions.

## Scope and Precedence

Open Interpreter loads:

| Scope   | Path                                                                             |
| ------- | -------------------------------------------------------------------------------- |
| Global  | `~/.openinterpreter/AGENTS.md`                                                   |
| Project | `AGENTS.md` files from the repository root down to the current working directory |

More specific files override or supplement broader ones. A file closer to your
current directory is usually more relevant than one near the root.

## Temporary Override

Create this file to replace the global instructions for local testing:

```text
~/.openinterpreter/AGENTS.override.md
```

Delete it to return to the normal global file.

## Size

The combined project instructions are capped by `project_doc_max_bytes`.
Directory-specific files are prioritized so nearby guidance survives when the
limit is reached.

## Example

```markdown
# Project Instructions

## Commands

- `pnpm test` runs unit tests.
- `pnpm lint` must pass before final changes.
- Use `pnpm typecheck` after editing TypeScript types.

## Conventions

- Keep server code under `src/server`.
- Keep UI components small and colocated with their tests.
- Prefer existing helpers in `src/lib`.

## Cautions

- Do not edit generated files under `src/generated`.
- Ask before changing database migrations.
```

Good `AGENTS.md` files are short, specific, and durable. Put temporary task
details in the prompt, not in project instructions.
