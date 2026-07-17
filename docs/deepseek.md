---
title: DeepSeek
description: Use DeepSeek models with the default Claude Code bare harness or the optional DeepSeek TUI harness.
---

Open Interpreter connects directly to the
[DeepSeek API](https://api-docs.deepseek.com/) with the built-in `deepseek`
provider. It uses DeepSeek's OpenAI-compatible Chat endpoint and reads your key
from `DEEPSEEK_API_KEY`.

## Start With DeepSeek

Create a key in the DeepSeek platform, export it, and start Open Interpreter:

```bash
export DEEPSEEK_API_KEY="..."
interpreter
```

Open `/model`, select **DeepSeek**, then select a model. For a direct launch:

```bash
DEEPSEEK_API_KEY="..." interpreter \
  -c 'model_provider="deepseek"' \
  -m deepseek-v4-pro
```

For one non-interactive task:

```bash
DEEPSEEK_API_KEY="..." interpreter exec \
  -c 'model_provider="deepseek"' \
  -m deepseek-v4-pro \
  "Review this repository and fix the highest-impact bug."
```

## Choose a Model

The `/model` picker is generated from maintained provider catalogs and the
provider's current model data. Use it as the source of truth for models
available in your installed version.

DeepSeek's current API model IDs are `deepseek-v4-pro` and
`deepseek-v4-flash`. Use Pro for the higher-capacity option and Flash when cost
or concurrency matters more. DeepSeek has announced that the legacy
`deepseek-chat` and `deepseek-reasoner` IDs will be discontinued on July 24,
2026, so new configurations should use the V4 IDs.
See DeepSeek's [API updates](https://api-docs.deepseek.com/updates/) for current
availability and its [pricing page](https://api-docs.deepseek.com/quick_start/pricing)
before committing to a workload.

## Harness Behavior

DeepSeek models use `claude-code-bare` automatically when you have not set a
harness. Open Interpreter carries that smaller Claude Code-shaped agent surface
over DeepSeek's Chat Completions endpoint; it does not run an external CLI.

To inspect or change the active harness, run `/harness`. The optional
`deepseek-tui` mode is available when you specifically want its DeepSeek
TUI/CodeWhale-shaped prompt and tools:

```toml
model_provider = "deepseek"
model = "deepseek-v4-pro"
harness = "deepseek-tui"
```

Remove the explicit `harness` line to return to the recommended automatic
default.

## Configuration

The built-in provider is equivalent to this connection:

```toml
model_provider = "deepseek"
model = "deepseek-v4-pro"

[model_providers.deepseek]
name = "DeepSeek"
base_url = "https://api.deepseek.com"
env_key = "DEEPSEEK_API_KEY"
wire_api = "chat"
```

You normally do not need to copy that provider block. It is useful when you
need to understand a proxy or custom deployment. If you change the endpoint,
keep `wire_api = "chat"` for DeepSeek's OpenAI-compatible Chat API.

## Editors and SDKs

The same provider configuration works through `interpreter acp` in
[ACP-compatible editors](/docs/acp) and through Open Interpreter's
[Codex SDK compatibility](/docs/sdk).

## Troubleshooting

- A 401 or 403 usually means `DEEPSEEK_API_KEY` is missing, expired, or belongs
  to a different endpoint.
- If a legacy model ID stops working, open `/model` and select a current V4
  model instead of hard-coding the replacement.
- If the agent surface is unexpected, run `/harness` and remove any old
  explicit `harness` value from `~/.openinterpreter/config.toml`.
- If a proxy exposes a different protocol, match its `wire_api`; do not assume
  every DeepSeek-compatible endpoint has the same transport.
