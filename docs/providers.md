---
title: Providers and models
description: Pick the model that fits the task, from any provider.
---

Open Interpreter is provider agnostic. The provider you choose decides
which models are available and where the bill goes. You can mix providers
across sessions and even switch mid-conversation.

## Built-in providers

| Provider   | What it gives you                                           |
| ---------- | ----------------------------------------------------------- |
| OpenAI     | GPT-5 family including the Codex variants. Default pick.    |
| Anthropic  | Claude family. Strong at long-running, careful work.        |
| Ollama     | Local models, fully offline. Discovers what you installed.  |
| LM Studio  | Local server with a friendly GUI for model management.      |

## Add a custom provider

Anything OpenAI-compatible works. Pick "Add a custom provider" in the
onboarding picker, or add it to `config.toml` directly:

```toml
[model_providers.together]
name = "Together"
base_url = "https://api.together.xyz/v1"
env_key = "TOGETHER_API_KEY"
wire_api = "openai"

[profiles.together-llama]
model_provider = "together"
model = "meta-llama/Llama-3.3-70B-Instruct-Turbo"
```

Then:

```bash
interpreter --profile together-llama
```

## Switch from inside a session

```
/model
```

Pick a different provider, model, or reasoning effort. The current model
shows in the footer.

For the fastest responses on a supported model:

```
/fast
```

## Reasoning effort

Open Interpreter exposes a reasoning dial for models that support it.

| Level    | When to use                                       |
| -------- | ------------------------------------------------- |
| `none`   | Quick edits, simple lookups                       |
| `low`    | Routine work where speed matters                  |
| `medium` | Default. Balanced for everyday use                |
| `high`   | Tricky bugs, complex refactors, long-form review  |

Set it as a default in `config.toml`:

```toml
model_reasoning_effort = "medium"
```

Or change it in the `/model` picker.

## Local models

Local models keep everything on your machine. Useful for sensitive code
and for working offline.

<Steps>
  <Step title="Install a runner">
    Grab [Ollama](https://ollama.com/) or [LM Studio](https://lmstudio.ai/).
  </Step>
  <Step title="Pull a model">
    For Ollama:
    ```bash
    ollama pull qwen2.5-coder:14b
    ```
  </Step>
  <Step title="Pick it in Open Interpreter">
    Run `/model` and choose the local provider. Open Interpreter
    discovers what is installed.
  </Step>
</Steps>

<Tip>
Smaller local models work well for in-context edits and review. Reach
for a frontier model when you need long-horizon planning or careful
debugging.
</Tip>
