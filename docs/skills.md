---
title: Skills
description: Package reusable workflows the agent can pick up automatically.
---

A skill is a folder with a `SKILL.md` and any helper files the workflow
needs. When a request matches the skill description, Open Interpreter
loads it and follows the instructions.

Use skills when the same task keeps coming up and you want it to run the
same way every time. Things like:

- "Cut a release"
- "Run the migration checklist"
- "Generate an end-of-week report"

## Anatomy of a skill

```
my-skill/
├── SKILL.md          # required: instructions and metadata
├── scripts/          # optional: executable helpers
├── references/       # optional: docs the agent can cite
└── assets/           # optional: templates the agent fills in
```

A minimal `SKILL.md`:

```markdown
---
name: cut-release
description: Cut a tagged release and update the changelog.
---

When the user asks to cut a release:

1. Run `pnpm test` and bail out if anything fails.
2. Bump the version in `package.json` using semver based on the changes.
3. Move the "Unreleased" section in `CHANGELOG.md` under a new heading.
4. Commit, tag, and push.
```

The frontmatter `description` is what Open Interpreter matches against,
so make it specific.

## Where skills live

Open Interpreter searches a list of locations and uses the first match.

| Path                                    | Scope                  |
| --------------------------------------- | ---------------------- |
| `.agents/skills/` in the current directory | Repo-local           |
| `.agents/skills/` in any parent          | Folder-specific shared |
| `.agents/skills/` at the repo root      | Org-wide for the repo  |
| `~/.agents/skills/`                     | Personal, all repos    |
| Bundled skills                          | Always available       |

Repo-local skills win over personal ones. Personal skills win over
bundled defaults.

## Use a skill

You usually do not have to do anything. The agent checks descriptions
against the request and pulls in matching skills automatically.

To browse what is available or trigger one explicitly:

```
/skills
```

## Write a new skill

<Steps>
  <Step title="Decide where it lives">
    Personal? Put it under `~/.agents/skills/`. For your team? Commit it
    to `.agents/skills/` in the repo.
  </Step>
  <Step title="Make the folder">
    ```bash
    mkdir -p .agents/skills/cut-release
    cd .agents/skills/cut-release
    ```
  </Step>
  <Step title="Write SKILL.md">
    Start with a sharp description. The agent uses it to decide whether
    your skill applies. Vague descriptions get picked up at the wrong
    times or never at all.
  </Step>
  <Step title="Add helpers">
    Drop scripts under `scripts/` and reference them by relative path
    from the instructions. The agent runs them through the normal
    sandbox and approval rules.
  </Step>
  <Step title="Try it">
    Open a session and trigger the workflow. Refine the description and
    instructions until the agent reaches for the skill at the right
    moment.
  </Step>
</Steps>

<Tip>
A good skill reads like an SOP for a careful coworker. Short, specific,
and explicit about what to check.
</Tip>
