---
title: Quickstart
description: Run your first Open Interpreter session in under a minute.
---

<Steps>
  <Step title="Install">
    ```bash
    curl -fsSL https://openinterpreter.com/install.sh | sh
    ```

    See [Install](/docs/install) for Windows and other options.
  </Step>
  <Step title="Open a project">
    ```bash
    cd my-project
    interpreter
    ```
  </Step>
  <Step title="Pick a provider">
    The first run walks you through provider setup. Choose ChatGPT, an API
    key, or a local model. You can change this any time with `/model`.

    See [Authentication](/docs/authentication) for the full list.
  </Step>
  <Step title="Ask for something">
    Type a request in plain English:

    ```
    > add a /health endpoint that returns the build sha
    ```

    Open Interpreter reads the relevant files, plans the change, and shows
    you the diff before it edits anything.
  </Step>
  <Step title="Approve actions">
    When the agent wants to run a command that needs approval, it shows
    you the command and waits. Press `y` to approve, `a` to approve and
    not ask again for that command this session, or `n` (or `Esc`) to
    deny.

    Want fewer prompts? Switch the [approval mode](/docs/sandbox) with
    `/permissions`.
  </Step>
  <Step title="Resume later">
    When you come back tomorrow, run:

    ```bash
    interpreter resume --last
    ```

    The previous conversation, files, and context come right back.
  </Step>
</Steps>

## What to try next

<CardGroup cols={2}>
  <Card title="Write an AGENTS.md" href="/docs/agents_md">
    Tell the agent how your project works, once.
  </Card>
  <Card title="Use slash commands" href="/docs/slash_commands">
    `/model`, `/diff`, `/review`, `/plan`, and more.
  </Card>
  <Card title="Run non-interactively" href="/docs/exec">
    Pipe Open Interpreter into scripts and CI.
  </Card>
  <Card title="Connect MCP servers" href="/docs/mcp">
    Give the agent access to external tools and data.
  </Card>
</CardGroup>
