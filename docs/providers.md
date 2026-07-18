---
title: Model Providers
description: Choose a provider, connect an account or API key, and use the right harness for its models.
---

Providers connect Open Interpreter to model services. A provider decides where
requests are sent and how you authenticate; a model is the model ID you run; a
harness controls the agent-facing prompt, tools, and message behavior.

For most providers, start Open Interpreter and use `/model`:

```text
> /model
```

Choose the provider, authenticate or set the requested environment variable,
then choose a model. Open Interpreter automatically selects a provider-specific
harness when one is configured for that model family. You can inspect or
override it with `/harness`.

## Provider Guides

| Provider family | Authentication | Default behavior | Guide |
| --- | --- | --- | --- |
| Kimi K3 and Moonshot | Kimi Code sign-in, `KIMI_API_KEY`, or `MOONSHOT_API_KEY` | `kimi-code` harness | [Kimi K3](/docs/kimi-k3) |
| DeepSeek | `DEEPSEEK_API_KEY` | `claude-code-bare` harness | [DeepSeek](/docs/deepseek) |
| Z.AI, Zhipu AI, and GLM | `ZAI_API_KEY` or `ZHIPU_API_KEY` | Generic Chat, or `zcode` with a Messages endpoint | [Z.AI, GLM, and ZCode](/docs/zai-glm) |

These guides cover the provider IDs, current model-selection path, API or
subscription setup, harness behavior, direct CLI use, and common mistakes.

## Provider, Model, and Harness

Keep the three layers separate when troubleshooting:

| Layer | Example | What it controls |
| --- | --- | --- |
| Provider | `deepseek` | Endpoint, credentials, and wire API |
| Model | `deepseek-v4-pro` | The model sent to that endpoint |
| Harness | `claude-code-bare` | Agent prompt, tools, and request shaping |

The provider's `wire_api` must support the selected harness. See
[Harness](/docs/harness) for the compatibility matrix.

## Provider Reference

The active provider decides where requests are sent, how credentials are
attached, which wire API is used, and which bundled model metadata seeds the
picker.

The source of truth is code, not this table:

- Built-in runtime providers: `codex-rs/model-provider-info/src/lib.rs`
- Generated hosted-provider catalog:
  `codex-rs/model-provider-info/provider_catalog.json`
- Hosted-provider generator inputs:
  `codex-rs/scripts/write_provider_catalog.py` and
  `codex-rs/model-provider-info/provider_catalog_overrides.json`
- Model metadata: `codex-rs/codex-api/model_compatibility_catalog.json`
  and `codex-rs/models-manager/src/provider_catalog_models.rs`

## Built-In Runtime Providers

These providers exist even before the generated hosted-provider catalog is
consulted.

| Provider ID | Name | Wire API | Auth |
| --- | --- | --- | --- |
| `openai` | OpenAI | `responses` | ChatGPT sign-in or OpenAI auth manager |
| `amazon-bedrock` | Amazon Bedrock | `responses` | AWS SigV4 via `model_providers.amazon-bedrock.aws` |
| `ollama` | Ollama local OSS | `responses` | none |
| `lmstudio` | LM Studio local OSS | `responses` | none |

OpenAI also has an onboarding preset named `openai_api_key`. That preset writes
an OpenAI-compatible provider using `OPENAI_API_KEY`; it is a setup shortcut,
not a separate built-in provider id in the runtime provider map.

## Wire APIs

`wire_api` controls the HTTP request shape.

| Value | Transport | Used by |
| --- | --- | --- |
| `responses` | OpenAI Responses API style requests | OpenAI, Bedrock, Ollama, LM Studio, custom Responses-compatible providers |
| `chat` | OpenAI-compatible Chat Completions requests | Most generated hosted providers and chat harnesses |
| `messages` | Anthropic Messages requests | Anthropic-style providers and Claude Code harness mode |

For custom providers:

```toml
[model_providers.example]
name = "Example"
base_url = "https://api.example.com/v1"
env_key = "EXAMPLE_API_KEY"
wire_api = "chat"
```

Use `wire_api = "responses"` for OpenAI Responses-compatible providers,
`wire_api = "chat"` for OpenAI-compatible chat-completions providers, and
`wire_api = "messages"` only for Anthropic Messages-compatible providers.

For example, the hosted [app.nz](https://app.nz/docs) gateway is an
OpenAI-compatible chat-completions provider whose `app/auto` model auto-routes
across upstream providers:

```toml
[model_providers.appnz]
name = "app.nz"
base_url = "https://app.nz/v1"
env_key = "APPNZ_API_KEY"
wire_api = "chat"
```

## Authentication

Provider auth can come from:

- `env_key`, which reads a bearer token from an environment variable.
- `experimental_bearer_token`, when embedding a token is unavoidable.
- command-backed `auth`, which runs a local command and caches the returned
  bearer token.
- `aws`, only for `amazon-bedrock`.
- OpenAI auth manager state for providers that set `requires_openai_auth`.

Anthropic-style API-key auth uses the `x-api-key` header and automatically adds
`anthropic-version: 2023-06-01`. Other API-key providers use
`Authorization: Bearer ...`.

Kimi has two distinct provider paths:

- `kimi-for-coding` uses `https://api.kimi.com/coding/v1` and supports the
  built-in Kimi sign-in flow for an eligible Kimi Code subscription. It can
  also read a compatible token from `KIMI_API_KEY`.
- `moonshotai` uses `https://api.moonshot.ai/v1` with a Moonshot Platform API
  key from `MOONSHOT_API_KEY`. A Moonshot Platform key is not a Kimi Code
  subscription token.

Command-backed auth example:

```toml
[model_providers.example.auth]
command = "example-token"
args = ["print"]
timeout_ms = 5000
refresh_interval_ms = 300000
```

## Generated Hosted Providers

The generated catalog is built from `https://models.dev/api.json`, plus live
provider model endpoints configured in
`codex-rs/model-provider-info/provider_catalog_overrides.json`. The generator
includes providers backed by supported AI SDK packages, excludes unsupported or
local-only entries, requires a usable base URL, and keeps only models that
advertise tool calling and text output. Some provider entries are further
filtered against live model IDs so decommissioned upstream models do not remain
visible.

Common generated providers include:

| Provider ID | Name | Wire API | Auth env | Models |
| --- | --- | --- | --- | ---: |
| `anthropic` | Anthropic | `messages` | `ANTHROPIC_API_KEY` | 25 |
| `openrouter` | OpenRouter | `chat` | `OPENROUTER_API_KEY` | 252 |
| `groq` | Groq | `chat` | `GROQ_API_KEY` | 7 |
| `github-models` | GitHub Models | `chat` | `GITHUB_TOKEN` | 49 |
| `opencode` | OpenCode Zen | `chat` | `OPENCODE_API_KEY` | 70 |
| `opencode-go` | OpenCode Go | `chat` | `OPENCODE_API_KEY` | 18 |
| `github-copilot` | GitHub Copilot | `chat` | `GITHUB_TOKEN` | 23 |
| `poe` | Poe | `chat` | `POE_API_KEY` | 103 |
| `perplexity-agent` | Perplexity Agent | `chat` | `PERPLEXITY_API_KEY` | 18 |
| `requesty` | Requesty | `chat` | `REQUESTY_API_KEY` | 37 |
| `deepseek` | DeepSeek | `chat` | `DEEPSEEK_API_KEY` | 4 |
| `moonshotai` | Moonshot AI | `chat` | `MOONSHOT_API_KEY` | 17 |
| `moonshotai-cn` | Moonshot AI (China) | `chat` | `MOONSHOT_API_KEY` | 7 |
| `zhipuai` | Zhipu AI | `chat` | `ZHIPU_API_KEY` | 12 |
| `zai` | Z.AI | `chat` | `ZAI_API_KEY` | 14 |
| `siliconflow` | SiliconFlow | `chat` | `SILICONFLOW_API_KEY` | 76 |
| `siliconflow-cn` | SiliconFlow (China) | `chat` | `SILICONFLOW_CN_API_KEY` | 77 |
| `alibaba` | Alibaba | `chat` | `DASHSCOPE_API_KEY` | 43 |
| `alibaba-cn` | Alibaba (China) | `chat` | `DASHSCOPE_API_KEY` | 75 |
| `stepfun` | StepFun | `chat` | `STEPFUN_API_KEY` | 4 |
| `modelscope` | ModelScope | `chat` | `MODELSCOPE_API_KEY` | 7 |
| `qiniu-ai` | Qiniu | `chat` | `QINIU_API_KEY` | 81 |
| `jiekou` | Jiekou.AI | `chat` | `JIEKOU_API_KEY` | 58 |
| `302ai` | 302.AI | `chat` | `302AI_API_KEY` | 92 |
| `novita-ai` | NovitaAI | `chat` | `NOVITA_API_KEY` | 64 |
| `fireworks-ai` | Fireworks AI | `chat` | `FIREWORKS_API_KEY` | 20 |
| `nvidia` | Nvidia | `chat` | `NVIDIA_API_KEY` | 52 |
| `huggingface` | Hugging Face | `chat` | `HF_TOKEN` | 22 |
| `chutes` | Chutes | `chat` | `CHUTES_API_KEY` | 30 |
| `ollama-cloud` | Ollama Cloud | `chat` | `OLLAMA_API_KEY` | 36 |
| `llama` | Llama | `chat` | `LLAMA_API_KEY` | 7 |
| `upstage` | Upstage | `chat` | `UPSTAGE_API_KEY` | 3 |
| `nova` | Nova | `chat` | `NOVA_API_KEY` | 2 |
| `xiaomi` | Xiaomi | `chat` | `XIAOMI_API_KEY` | 5 |
| `abacus` | Abacus | `chat` | `ABACUS_API_KEY` | 65 |
| `abliteration-ai` | abliteration.ai | `chat` | `ABLIT_KEY` | 1 |
| `alibaba-coding-plan` | Alibaba Coding Plan | `chat` | `ALIBABA_CODING_PLAN_API_KEY` | 9 |
| `alibaba-coding-plan-cn` | Alibaba Coding Plan (China) | `chat` | `ALIBABA_CODING_PLAN_API_KEY` | 9 |
| `ambient` | Ambient | `chat` | `AMBIENT_API_KEY` | 2 |
| `auriko` | Auriko | `chat` | `AURIKO_API_KEY` | 15 |
| `bailing` | Bailing | `chat` | `BAILING_API_TOKEN` | 1 |
| `baseten` | Baseten | `chat` | `BASETEN_API_KEY` | 14 |
| `berget` | Berget.AI | `chat` | `BERGET_API_KEY` | 7 |
| `clarifai` | Clarifai | `chat` | `CLARIFAI_PAT` | 10 |
| `claudinio` | Claudinio | `chat` | `CLAUDINIO_API_KEY` | 1 |
| `cloudferro-sherlock` | CloudFerro Sherlock | `chat` | `CLOUDFERRO_SHERLOCK_API_KEY` | 5 |
| `cortecs` | Cortecs | `chat` | `CORTECS_API_KEY` | 48 |
| `drun` | D.Run (China) | `chat` | `DRUN_API_KEY` | 3 |
| `digitalocean` | DigitalOcean | `chat` | `DIGITALOCEAN_ACCESS_TOKEN` | 59 |
| `dinference` | DInference | `chat` | `DINFERENCE_API_KEY` | 5 |
| `evroc` | evroc | `chat` | `EVROC_API_KEY` | 5 |
| `fastrouter` | FastRouter | `chat` | `FASTROUTER_API_KEY` | 14 |
| `firepass` | Fireworks (Firepass) | `chat` | `FIREPASS_API_KEY` | 1 |
| `friendli` | Friendli | `chat` | `FRIENDLI_TOKEN` | 6 |
| `frogbot` | FrogBot | `chat` | `FROGBOT_API_KEY` | 26 |
| `gmicloud` | GMI Cloud | `chat` | `GMICLOUD_API_KEY` | 8 |
| `helicone` | Helicone | `chat` | `HELICONE_API_KEY` | 71 |
| `hpc-ai` | HPC-AI | `chat` | `HPC_AI_API_KEY` | 3 |
| `iflowcn` | iFlow | `chat` | `IFLOW_API_KEY` | 14 |
| `inception` | Inception | `chat` | `INCEPTION_API_KEY` | 1 |
| `inference` | Inference | `chat` | `INFERENCE_API_KEY` | 8 |
| `io-net` | IO.NET | `chat` | `IOINTELLIGENCE_API_KEY` | 17 |
| `kilo` | Kilo Gateway | `chat` | `KILO_API_KEY` | 258 |
| `kimi-for-coding` | Kimi For Coding | `chat` | `KIMI_API_KEY` | 6 |
| `kuae-cloud-coding-plan` | KUAE Cloud Coding Plan | `chat` | `KUAE_API_KEY` | 1 |
| `lilac` | Lilac | `chat` | `LILAC_API_KEY` | 4 |
| `llmgateway` | LLM Gateway | `chat` | `LLMGATEWAY_API_KEY` | 156 |
| `lucidquery` | LucidQuery AI | `chat` | `LUCIDQUERY_API_KEY` | 2 |
| `meganova` | Meganova | `chat` | `MEGANOVA_API_KEY` | 18 |
| `minimax` | MiniMax (minimax.io) | `messages` | `MINIMAX_API_KEY` | 6 |
| `minimax-cn` | MiniMax (minimaxi.com) | `messages` | `MINIMAX_API_KEY` | 6 |
| `minimax-coding-plan` | MiniMax Token Plan (minimax.io) | `messages` | `MINIMAX_API_KEY` | 6 |
| `minimax-cn-coding-plan` | MiniMax Token Plan (minimaxi.com) | `messages` | `MINIMAX_API_KEY` | 6 |
| `mixlayer` | Mixlayer | `chat` | `MIXLAYER_API_KEY` | 5 |
| `moark` | Moark | `chat` | `MOARK_API_KEY` | 2 |
| `nano-gpt` | NanoGPT | `chat` | `NANO_GPT_API_KEY` | 105 |
| `nearai` | NEAR AI Cloud | `chat` | `NEARAI_API_KEY` | 29 |
| `nebius` | Nebius Token Factory | `chat` | `NEBIUS_API_KEY` | 29 |
| `neuralwatt` | Neuralwatt | `chat` | `NEURALWATT_API_KEY` | 14 |
| `orcarouter` | OrcaRouter | `chat` | `ORCAROUTER_API_KEY` | 79 |
| `ovhcloud` | OVHcloud AI Endpoints | `chat` | `OVHCLOUD_API_KEY` | 10 |
| `qihang-ai` | QiHang | `chat` | `QIHANG_API_KEY` | 9 |
| `regolo-ai` | Regolo AI | `chat` | `REGOLO_API_KEY` | 10 |
| `routing-run` | routing.run | `chat` | `ROUTING_RUN_API_KEY` | 24 |
| `sarvam` | Sarvam AI | `chat` | `SARVAM_API_KEY` | 2 |
| `scaleway` | Scaleway | `chat` | `SCALEWAY_API_KEY` | 14 |
| `stackit` | STACKIT | `chat` | `STACKIT_API_KEY` | 5 |
| `submodel` | submodel | `chat` | `SUBMODEL_INSTAGEN_ACCESS_KEY` | 9 |
| `synthetic` | Synthetic | `chat` | `SYNTHETIC_API_KEY` | 32 |
| `tencent-coding-plan` | Tencent Coding Plan (China) | `chat` | `TENCENT_CODING_PLAN_API_KEY` | 8 |
| `tencent-tokenhub` | Tencent TokenHub | `chat` | `TENCENT_TOKENHUB_API_KEY` | 1 |
| `the-grid-ai` | The Grid AI | `chat` | `THEGRIDAI_API_KEY` | 9 |
| `umans-ai-coding-plan` | Umans AI Coding Plan | `chat` | `UMANS_AI_CODING_PLAN_API_KEY` | 5 |
| `vivgrid` | Vivgrid | `chat` | `VIVGRID_API_KEY` | 13 |
| `vultr` | Vultr | `chat` | `VULTR_API_KEY` | 5 |
| `wafer.ai` | Wafer | `chat` | `WAFER_API_KEY` | 2 |
| `wandb` | Weights & Biases | `chat` | `WANDB_API_KEY` | 18 |
| `xiaomi-token-plan-cn` | Xiaomi Token Plan (China) | `chat` | `XIAOMI_API_KEY` | 5 |
| `xiaomi-token-plan-ams` | Xiaomi Token Plan (Europe) | `chat` | `XIAOMI_API_KEY` | 5 |
| `xiaomi-token-plan-sgp` | Xiaomi Token Plan (Singapore) | `chat` | `XIAOMI_API_KEY` | 4 |
| `xpersona` | Xpersona | `chat` | `XPERSONA_API_KEY` | 2 |
| `zai-coding-plan` | Z.AI Coding Plan | `chat` | `ZAI_API_KEY` | 6 |
| `zenmux` | ZenMux | `chat` | `ZENMUX_API_KEY` | 110 |
| `zhipuai-coding-plan` | Zhipu AI Coding Plan | `chat` | `ZHIPU_API_KEY` | 7 |

## Model Picker Behavior

The picker uses provider metadata in two passes:

1. It asks the active provider's `/models` endpoint when possible.
2. It seeds or supplements results from the bundled provider catalog when the
   provider matches by Anthropic identity, base URL, provider name, or auth env
   var.

Generated model entries carry display name, description, context window,
reasoning control, input modalities, and priority. The generator does not
invent reasoning levels from a boolean reasoning flag; those controls come
from compatibility metadata or explicit overrides.

## Default Harness Selection

When no harness is explicitly configured, Open Interpreter may choose a native
harness mode from the provider/model family:

| Match | Default harness |
| --- | --- |
| `wire_api = "messages"`, Anthropic provider/name/base URL, or `claude` model ids | `claude-code` |
| `kimi`, `moonshot`, `api.kimi.com`, `api.moonshot.ai`, or `api.moonshot.cn` | `kimi-code` |
| `qwen`, `qwq`, `dashscope`, or DashScope compatible-mode base URLs | `qwen-code` |
| `deepseek` or `api.deepseek.com` | `claude-code-bare` |

You can override this with `harness = "..."` in config. See
[Harness](/docs/harness) for route compatibility.

## Regenerating The Catalog

Run this from `codex-rs` when provider source data changes:

```bash
python3 scripts/write_provider_catalog.py
python3 scripts/write_model_compatibility_catalog.py
```

To refresh only selected hosted providers while preserving every other
generated entry, repeat `--provider` as needed:

```bash
python3 scripts/write_provider_catalog.py \
  --provider moonshotai \
  --provider kimi-for-coding
```

Live sources require their documented provider auth environment variables. For
example, Moonshot requires `MOONSHOT_API_KEY` or `KIMI_API_KEY`, Z.AI requires
`ZAI_API_KEY` or `ZHIPU_API_KEY`, and Groq requires `GROQ_API_KEY`. Missing
auth is a generator failure, not a reason to silently keep stale data.
