---
title: AGENTS.md
description: Give Open Interpreter project-specific instructions it always reads.
---

`AGENTS.md` is a Markdown file you keep in your repo. Open Interpreter
reads it before every session and treats it as the team norms for that
project.

Use it for the things you would otherwise repeat in every prompt:

- Build and test commands the agent should prefer
- Lint and formatting rules
- Conventions you do not want broken
- Directories the agent should be careful with
- Anything a new contributor would want to know on day one

## Create one fast

Run this from inside a session and the agent generates a starting
`AGENTS.md` based on what it sees in the repo:

```
/init
```

Edit the result to match how your team actually works.

## Where it lives

Open Interpreter looks for `AGENTS.md` in two places:

| Scope     | Path                                                 |
| --------- | ---------------------------------------------------- |
| Global    | `~/.openinterpreter/AGENTS.md`                       |
| Project   | Every directory from the Git root down to your `cwd` |

Files closer to your working directory take priority over files higher
up. The global file is the lowest priority and applies everywhere.

## Override temporarily

Need to swap the global file for a one-off run? Drop in
`~/.openinterpreter/AGENTS.override.md`. While it exists, it replaces the
global `AGENTS.md`. Delete it to go back.

## Size limits

The combined contents are capped (32 KiB by default). Files closer to
your working directory are kept first, so directory-specific notes always
make it in. Adjust the cap with `project_doc_max_bytes` in `config.toml`
if you have a real reason to.

## A good starting point

```markdown
# Project notes

## How to build
- `pnpm install`
- `pnpm dev` for local
- `pnpm test` runs Jest plus Playwright

## Conventions
- TypeScript strict mode is on, no `any`
- Server code lives under `src/server`, client under `src/app`
- Database migrations are owned by the data team. Ask before adding one.

## Be careful
- Do not edit anything under `vendor/`
- The `scripts/release.sh` flow is run by CI, not locally
```

## Hierarchical guidance

When the `child_agents_md` feature flag is on, Open Interpreter also
emits guidance about scope and precedence even when no `AGENTS.md` is
present. Enable it under `[features]` in `config.toml`:

```toml
[features]
child_agents_md = true
```
