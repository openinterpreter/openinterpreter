---
title: Authentication
description: Sign in with a provider, use an API key, or point at a local model.
---

Open Interpreter is provider agnostic. The first time you run it, the
onboarding flow asks you to pick how you want to authenticate. You can
change this any time from a session with `/model`.

## Sign in with ChatGPT

If you have a paid ChatGPT plan, this is the simplest path. Open Interpreter
opens a browser window, you sign in, and tokens refresh automatically while
you use the CLI.

```bash
interpreter
# choose "Sign in with ChatGPT" in the picker
```

Use this when you want billing tied to your ChatGPT plan and the smoothest
setup.

## Use an API key

API keys are the right default for automation, CI, and headless setups.

<Tabs>
  <Tab title="OpenAI">
    ```bash
    export OPENAI_API_KEY=sk-...
    interpreter
    ```
  </Tab>
  <Tab title="Anthropic">
    ```bash
    export ANTHROPIC_API_KEY=sk-ant-...
    interpreter
    ```
  </Tab>
  <Tab title="Other providers">
    Pick "Add a custom provider" in the onboarding picker, paste the base
    URL and an API key, and Open Interpreter writes a provider entry into
    `~/.openinterpreter/config.toml`.
  </Tab>
</Tabs>

You can also paste a key directly into the provider picker. Open
Interpreter stores it in the system credential store when one is available
and falls back to a file under `~/.openinterpreter/`.

## Connect a local model

Open Interpreter ships with first-class support for local model runners.
Both keep everything on your machine, useful for sensitive work and for
running offline.

- **Ollama.** Pick "Ollama" in the picker. Open Interpreter discovers
  your installed models automatically.
- **LM Studio.** Pick "LM Studio". Make sure the local server is
  running on its default port.

## Where credentials live

| Item                    | Location                                    |
| ----------------------- | ------------------------------------------- |
| Cached login tokens     | `~/.openinterpreter/auth.json` or keyring   |
| Configured providers    | `~/.openinterpreter/config.toml`            |
| API keys (per provider) | Environment variables or `config.toml`      |

Treat the auth file like a password. Anyone with read access to it can use
your account.

To switch storage between a file and the system keyring, set
`cli_auth_credentials_store` in `config.toml`:

```toml
cli_auth_credentials_store = "keyring"   # or "file" or "auto"
```

## Sign out

```bash
/logout
```

Or remove the auth file directly:

```bash
rm ~/.openinterpreter/auth.json
```
