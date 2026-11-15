---
title: Quickstart
description: Install Open Interpreter, open a project, and run your first coding-agent session.
---

Open Interpreter is a terminal coding agent built from the Codex CLI surface and
adapted for provider-agnostic local use. It can inspect your repository, edit
files, run commands, review diffs, and resume work later.

<Steps>
  <Step title="Install">
    ```bash
    curl -fsSL https://www.openinterpreter.com/install | sh
    ```

    On Windows, run:

    ```powershell
    irm https://www.openinterpreter.com/install.ps1 | iex
    ```

  </Step>
  <Step title="Open a project">
    ```bash
    cd my-project
    i
    ```

    `i` is the short command for `interpreter`; either command starts an
    interactive session.

  </Step>
  <Step title="Choose a provider">
    The first run walks you through provider setup. You can sign in with
    ChatGPT, use an API key, connect a local model through Ollama or LM Studio,
    or configure another compatible provider.

    You can change providers later with `/model`.

  </Step>
  <Step title="Ask for a change">
    Type a concrete request:

    ```text
    add a /health endpoint that returns the build sha
    ```

    Open Interpreter reads the project, proposes work, edits files, and runs
    commands through the active sandbox and approval settings.

  </Step>
  <Step title="Approve actions">
    In the default local workflow, Open Interpreter can work inside the current
    workspace and asks before actions that need more access. Use `/permissions`
    to inspect or change this during a session.
  </Step>
  <Step title="Resume later">
    ```bash
    interpreter resume --last
    ```

    Resume keeps the prior conversation, context, and working directory.

  </Step>
</Steps>

## Common first commands

| Task                      | Command                                              |
| ------------------------- | ---------------------------------------------------- |
| Start the TUI             | `i` or `interpreter`                                 |
| Start with a prompt       | `interpreter "explain this repo"`                    |
| Run once without the TUI  | `interpreter exec "summarize the current diff"`      |
| Continue the last session | `interpreter resume --last`                          |
| Review current changes    | `/review` or `interpreter exec review --uncommitted` |
| Choose model/provider     | `/model`                                             |
| Change permissions        | `/permissions`                                       |

## Common setup answers

| If you need to…                                              | Go here                                               |
| ------------------------------------------------------------ | ----------------------------------------------------- |
| Verify your installed version, update, or uninstall          | [Install](/docs/install)                              |
| Connect Ollama or LM Studio, including a remote local server | [Local models](/docs/models#local-models)             |
| Connect a hosted model provider                              | [Providers](/docs/providers)                          |
| Set an API key or understand where credentials are stored    | [Authentication](/docs/authentication)                |
| Configure command, filesystem, and network boundaries        | [Sandbox & approvals](/docs/sandbox)                  |
| Embed Open Interpreter in an application                     | [SDK](/docs/sdk)                                      |
| Build a small local streaming browser chat                   | [A tiny streaming web chat](/docs/streaming-web-chat) |

## Next pages

<CardGroup cols={2}>
  <Card title="Interactive mode" href="/docs/interactive">
    Composer, shortcuts, images, prompts, approvals, review, and background work.
  </Card>
  <Card title="Configuration" href="/docs/config">
    Defaults, profiles, providers, model settings, feature flags, and project config.
  </Card>
  <Card title="Sandbox & approvals" href="/docs/sandbox">
    How local command execution is constrained and when Open Interpreter asks.
  </Card>
  <Card title="AGENTS.md" href="/docs/agents_md">
    Durable project instructions for build commands, conventions, and cautions.
  </Card>
</CardGroup>
