---
title: Models
description: How Open Interpreter lists models and surfaces model capabilities.
---

Use `/model` to choose provider, model, harness, and model-specific controls.
The footer shows the active selection.

## Shell Overrides

```bash
interpreter -m gpt-5.1-codex "review this module"
interpreter --oss "use my local open source provider"
```

## Config Defaults

```toml
model_provider = "openai"
model = "gpt-5.1-codex"
model_reasoning_effort = "medium"
model_reasoning_summary = "auto"
model_verbosity = "medium"
```

## Where Model Metadata Comes From

Open Interpreter does not keep one hand-written Rust list of every model.
Metadata is layered:

| Source | Role |
| --- | --- |
| Provider `/models` endpoint | Live model ids for the active provider when the endpoint is available. |
| `model-provider-info/provider_catalog.json` | Bundled provider/model seed data generated from `models.dev` and configured live provider model sources. |
| `codex-api/model_compatibility_catalog.json` | Compatibility metadata such as supported parameters, search support, reasoning levels, and input modalities. |
| `models-manager/models.json` | OpenAI-style model preset metadata used by the manager. |
| Config `model_catalog` | Optional user-supplied static catalog for a provider/session. |

The model manager asks the active provider for models, then uses bundled data
when it can identify the provider by Anthropic identity, base URL, provider
name, or auth env var. This lets proxy configurations still inherit useful
metadata when they clearly point at a known provider.

## Capability Metadata

Model metadata can control:

- picker visibility;
- display name and description;
- context window;
- input modalities such as text and image;
- whether the model is supported by the API;
- supported request parameters;
- reasoning control shape;
- web/search support;
- parallel tool-call support.

Reasoning is not a single boolean in the UI. The protocol has these control
shapes:

| Control | Meaning |
| --- | --- |
| `none` | No known reasoning control. |
| `fixed` | The model reasons, but the UI should not expose a control. |
| `effort` | OpenAI-style effort control. |
| `thinking_toggle` | Boolean thinking on/off control. |
| `thinking_budget` | Token-budget thinking control. |

## Reasoning Effort

When a model exposes effort controls, Open Interpreter uses these values:

| Value | Use for |
| --- | --- |
| `minimal` | Fast, simple edits. |
| `low` | Routine implementation. |
| `medium` | Default balanced work. |
| `high` | Hard debugging, refactors, reviews. |
| `xhigh` | Model-dependent extra reasoning. |

Harnesses may map these values to provider-specific fields. For example,
`kimi-cli` maps `minimal` and `low` to low reasoning, `medium` to medium, and
`high` or `xhigh` to high.

Unsupported models hide, ignore, or reject reasoning controls depending on
provider behavior.

## Input Modalities

The canonical input modality tags are:

| Value | Meaning |
| --- | --- |
| `text` | Normal user turns and tool payloads. |
| `image` | Image attachments from commands such as `-i`. |

Attach images:

```bash
interpreter -i screenshot.png "what is wrong here?"
```

Legacy payloads that omit modality metadata default conservatively to text and
image support for compatibility, but generated provider entries should specify
the real modalities when known.

## Local Models

Open Interpreter has two built-in local OSS providers:

| Provider | Default base URL | Override |
| --- | --- | --- |
| `ollama` | `http://localhost:11434/v1` | `CODEX_OSS_PORT` or `CODEX_OSS_BASE_URL` |
| `lmstudio` | `http://localhost:1234/v1` | `CODEX_OSS_PORT` or `CODEX_OSS_BASE_URL` |

Start the local service, then choose it from onboarding or `/model`.
`--oss` selects the configured local provider from the command line.

## Provider Families And Harness Defaults

Some model families get a default harness when no explicit harness is set:

| Model/provider family | Default harness |
| --- | --- |
| Claude/Anthropic/Messages | `claude-code` |
| Kimi/Moonshot | `kimi-code` |
| Qwen/QwQ/DashScope | `qwen-code` |
| DeepSeek | `claude-code-bare` |

See [Harness](/docs/harness) for the wire-api compatibility matrix and
[Model providers](/docs/providers) for provider-specific setup guides.
