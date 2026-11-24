---
title: Authentication
description: Sign in with ChatGPT, use API keys, or connect local and compatible providers.
---

Open Interpreter is provider-agnostic. The first run asks how you want to
authenticate, and `/model` lets you change that later.

## ChatGPT Login

Start the TUI and choose ChatGPT sign-in:

```bash
interpreter
```

This opens a browser-based login flow and stores refreshable credentials in the
configured credential store.

## API Keys

API keys are the best fit for CI, headless machines, and explicit provider
billing.

```bash
export OPENAI_API_KEY=sk-...
interpreter
```

Other providers use their own environment variables, such as
`ANTHROPIC_API_KEY`, or the variable configured by their provider entry.

## Local Providers

Use local runners when you want model traffic to stay on your machine:

| Provider  | Notes                                                      |
| --------- | ---------------------------------------------------------- |
| Ollama    | Start Ollama and choose it from `/model`, or use `--oss`.  |
| LM Studio | Start the local server and choose LM Studio from `/model`. |

```bash
interpreter --oss "summarize this repo with my local model"
```

## Compatible Providers

Add an OpenAI-compatible provider in config:

```toml
model_provider = "acme"
model = "acme-coder"

[model_providers.acme]
name = "Acme"
base_url = "https://api.acme.example/v1"
env_key = "ACME_API_KEY"
wire_api = "responses"
```

Then:

```bash
export ACME_API_KEY=...
interpreter
```

## Credential Storage

```toml
cli_auth_credentials_store = "auto" # "auto" | "keyring" | "file"
```

Open Interpreter stores user state under `~/.openinterpreter/`. Treat
`auth.json` like a password if file storage is used.

## Sign Out

Inside the TUI:

```text
/logout
```

Or from a compatible login surface, use the logout command when available.
