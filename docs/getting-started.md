---
title: Open Interpreter
description: A terminal coding agent that runs in your projects, with the model and provider you choose.
---

Open Interpreter is a coding agent that lives in your terminal. Open a project,
type `interpreter`, and it reads your files, edits them, and runs commands
on your behalf.

It is built on top of Codex and stays provider agnostic, so you can plug in
OpenAI, Anthropic, a local model, or anything else.

<CardGroup cols={2}>
  <Card title="Install" href="/docs/install">
    Get the `interpreter` command on your machine.
  </Card>
  <Card title="Quickstart" href="/docs/quickstart">
    Run your first session in under a minute.
  </Card>
  <Card title="Authenticate" href="/docs/authentication">
    Sign in or connect a provider.
  </Card>
  <Card title="Configure" href="/docs/config">
    Tune providers, sandbox, and features.
  </Card>
</CardGroup>

## What it does

- **Edits code in place.** Ask for a change and review the diff before
  it lands.
- **Runs commands with explicit guardrails.** A sandbox decides what is
  even possible, and an approval mode decides when to ask first.
- **Brings your own model.** Pick the provider and model that fit the
  task. Switch any time with `/model`.
- **Resumes long sessions.** Pick up earlier work with
  `interpreter resume --last`.

## How it feels

```bash
$ cd my-project
$ interpreter
> add a /health endpoint that returns the build sha
```

Open Interpreter plans the change, edits the files, and asks before it
escalates beyond the current sandbox.

## Where to go next

<CardGroup cols={2}>
  <Card title="Slash commands" href="/docs/slash_commands">
    Switch models, manage sessions, and more from the composer.
  </Card>
  <Card title="AGENTS.md" href="/docs/agents_md">
    Give the agent project-specific guidance it always reads.
  </Card>
  <Card title="Skills" href="/docs/skills">
    Package reusable workflows the agent can pick up automatically.
  </Card>
  <Card title="Sandbox & approvals" href="/docs/sandbox">
    Decide what runs automatically and what asks first.
  </Card>
</CardGroup>
