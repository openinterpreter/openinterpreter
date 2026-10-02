---
title: 模型提供商
description: 选择提供商，连接账户或 API 密钥，并为其模型使用合适的 harness。
---

提供商将 Open Interpreter 连接到模型服务。提供商决定请求发送到何处以及如何进行身份验证；模型是你运行的模型 ID；harness 控制面向代理的提示、工具以及消息行为。

对于大多数提供商，启动 Open Interpreter 并使用 `/model`：

```text
> /model
```

选择一个提供商，进行身份验证或设置所需的环境变量，然后选择模型。Open Interpreter 会在为该模型系列配置了 provider‑specific harness 时自动选择相应的 harness。你可以使用 `/harness` 检查或覆盖它。

## 提供商指南

| 提供商系列 | 身份验证 | 默认行为 | 指南 |
| --- | --- | --- | --- |
| Kimi K3 与 Moonshot | Kimi Code 登录、`KIMI_API_KEY` 或 `MOONSHOT_API_KEY` | `kimi-code` harness | [Kimi K3](/docs/kimi-k3) |
| DeepSeek | `DEEPSEEK_API_KEY` | `claude-code-bare` harness | [DeepSeek](/docs/deepseek) |
| Z.AI、Zhipu AI 与 GLM | `ZAI_API_KEY` 或 `ZHIPU_API_KEY` | 通用 Chat，或使用 Messages 端点的 `zcode` | [Z.AI、GLM 与 ZCode](/docs/zai-glm) |

这些指南涵盖提供商 ID、当前模型选择路径、API 或订阅设置、harness 行为、直接 CLI 使用以及常见错误。

Google AI Studio 可通过提供商 `google` 使用。设置 `GEMINI_API_KEY`，然后在
`/model` 中选择 Google 和当前的 Gemini 模型。请求使用 Google 兼容 OpenAI
的 Chat Completions 端点以及通用 chat harness。

## 提供商、模型和 Harness

排查问题时请保持这三层分离：

| 层级 | 示例 | 它控制的内容 |
| --- | --- | --- |
| 提供商 | `deepseek` | 端点、凭证和 wire API |
| 模型 | `deepseek-v4-pro` | 发送到该端点的模型 |
| Harness | `claude-code-bare` | 智能体提示、工具和请求格式 |

提供商的 `wire_api` 必须支持所选的 harness。请参阅 [Harness](/docs/harness) 获取兼容性矩阵。

## 提供商参考

活动的提供商决定请求发送的目标、凭证的附加方式、使用的 wire API 以及用于初始化选择器的捆绑模型元数据。

真实信息来源于代码，而非此表格：

- 内置运行时提供商：`codex-rs/model-provider-info/src/lib.rs`
- 生成的托管提供商目录：
  `codex-rs/model-provider-info/provider_catalog.json`
- 托管提供商生成器输入：
  `codex-rs/scripts/write_provider_catalog.py` 和
  `codex-rs/model-provider-info/provider_catalog_overrides.json`
- 模型元数据：`codex-rs/codex-api/model_compatibility_catalog.json`
  和 `codex-rs/models-manager/src/provider_catalog_models.rs`

## 内置运行时提供商

这些提供商即使在查询生成的 hosted-provider 目录之前也已存在。

| 提供商 ID | 名称 | Wire API | 认证 |
| --- | --- | --- | --- |
| `openai` | OpenAI | `responses` | ChatGPT 登录或 OpenAI 身份验证管理器 |
| `amazon-bedrock` | Amazon Bedrock | `responses` | 通过 `model_providers.amazon-bedrock.aws` 使用 AWS SigV4 |
| `ollama` | Ollama 本地开源版 | `responses` | 无 |
| `lmstudio` | LM Studio 本地开源版 | `responses` | 无 |

OpenAI 还有一个名为 `openai_api_key` 的入门预设。该预设会使用 `OPENAI_API_KEY` 编写一个兼容 OpenAI 的提供商；它是一个设置快捷方式，而不是运行时提供商映射中的独立内置提供商 ID。

## Wire API

`wire_api` 控制 HTTP 请求的结构。

| 值 | 传输方式 | 使用者 |
| --- | --- | --- |
| `responses` | OpenAI Responses API 风格的请求 | OpenAI、Bedrock、Ollama、LM Studio 和自定义 Responses 兼容提供商 |
| `chat` | 兼容 OpenAI 的聊天完成请求 | 大多数生成的托管提供商和 chat harness |
| `messages` | Anthropic Messages 请求 | Anthropic 风格的提供商和 Claude Code harness 模式 |

对于自定义提供商：

```toml
[model_providers.example]
name = "Example"
base_url = "https://api.example.com/v1"
env_key = "EXAMPLE_API_KEY"
wire_api = "chat"
```

使用 `wire_api = "responses"` 为 OpenAI Responses‑compatible 提供商，`wire_api = "chat"` 为兼容 OpenAI 的 chat‑completions 提供商，`wire_api = "messages"` 仅用于兼容 Anthropic Messages 的提供商。

例如，托管的 [app.nz](https://app.nz/) 网关是兼容 OpenAI 的 chat-completions
提供商，其 `app/auto` 模型会在上游提供商之间自动路由：

```toml
[model_providers.appnz]
name = "app.nz"
base_url = "https://app.nz/v1"
env_key = "APPNZ_API_KEY"
wire_api = "chat"
```

[Cheaper Inference](https://cheaperinference.com/) 是一个托管的兼容 OpenAI 的
chat-completions 网关。每个模型的价格比其所属实验室的标价低 15–60%。其提供商
配置如下：

```toml
[model_providers.cheaperinference]
name = "Cheaper Inference"
base_url = "https://api.cheaperinference.com/v1"
env_key = "CHEAPER_INFERENCE_API_KEY"
wire_api = "chat"
```

## 身份验证

提供商的身份验证可以来源于：

- `env_key`，从环境变量读取 Bearer 令牌。
- `experimental_bearer_token`，在必须嵌入令牌时使用。
- 基于命令的 `auth`，运行本地命令并缓存返回的 Bearer 令牌。
- `aws`，仅适用于 `amazon-bedrock`。
- 对于设置了 `requires_openai_auth` 的提供商，使用 OpenAI 授权管理器的状态。

Anthropic 风格的 API 密钥身份验证使用 `x-api-key` 头，并自动添加 `anthropic-version: 2023-06-01`。其他 API 密钥提供商使用 `Authorization: Bearer ...`。

Kimi 有两条不同的提供商路径：

- `kimi-for-coding` 使用 `https://api.kimi.com/coding/v1`，并支持适用于符合条件的 Kimi Code 订阅的内置 Kimi 登录流程。它也可以从 `KIMI_API_KEY` 读取兼容的令牌。
- `moonshotai` 使用 `https://api.moonshot.ai/v1`，并使用来自 `MOONSHOT_API_KEY` 的 Moonshot Platform API 密钥。Moonshot Platform 密钥并不是 Kimi Code 订阅令牌。

基于命令的身份验证示例：

```toml
[model_providers.example.auth]
command = "example-token"
args = ["print"]
timeout_ms = 5000
refresh_interval_ms = 300000
```

## 生成的托管提供商

生成的目录来源于 `https://models.dev/api.json`，并加上在 `codex-rs/model-provider-info/provider_catalog_overrides.json` 中配置的实时提供商模型端点。生成器会包含由受支持的 AI SDK 包提供支持的提供商，排除不受支持或仅限本地的条目，需要可用的基础 URL，并仅保留声明支持工具调用和文本输出的模型。某些提供商条目会进一步根据实时模型 ID 过滤，以防已停用的上游模型仍可见。

常见的生成提供商包括：

| 提供商 ID | 名称 | Wire API | 认证环境变量 | 模型数 |
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
| `zai-zcode` | Z.AI ZCode | `messages` | `ZAI_API_KEY` | 4 |
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

## 模型选择器行为

选择器在两个阶段使用提供商元数据：

1. 在可能的情况下，它会请求活动提供商的 `/models` 端点。
2. 当提供商通过 Anthropic 身份、基础 URL、提供商名称或认证环境变量匹配时，它会从捆绑的提供商目录中种子或补充结果。

生成的模型条目包含显示名称、描述、上下文窗口、推理控制、输入模式和优先级。生成器不会根据布尔推理标志自行创造推理级别；这些控制来源于兼容性元数据或显式覆盖。

## 默认 Harness 选择

当未显式配置 harness 时，Open Interpreter 可能会从提供商/模型系列中选择本地 harness 模式：

| 匹配条件 | 默认 harness |
| --- | --- |
| `wire_api = "messages"`、Anthropic 提供商/名称/基础 URL，或 `claude` 模型 ID | `claude-code` |
| `kimi`、`moonshot`、`api.kimi.com`、`api.moonshot.ai` 或 `api.moonshot.cn` | `kimi-code` |
| `qwen`、`qwq`、`dashscope` 或 DashScope 兼容模式的基础 URL | `qwen-code` |
| `deepseek` 或 `api.deepseek.com` | `claude-code-bare` |

你可以在配置中使用 `harness = "..."` 覆盖此设置。请参阅 [Harness](/docs/harness) 获取路由兼容性信息。

## 重新生成目录

当提供商源数据更改时，从 `codex-rs` 运行以下命令：

```bash
python3 scripts/write_provider_catalog.py
python3 scripts/write_model_compatibility_catalog.py
```

要仅刷新选定的托管提供商并保留所有其他生成的条目，可根据需要重复使用 `--provider`：

```bash
python3 scripts/write_provider_catalog.py \
  --provider moonshotai \
  --provider kimi-for-coding
```

实时来源需要其文档中列出的提供商认证环境变量。例如，Moonshot 需要 `MOONSHOT_API_KEY` 或 `KIMI_API_KEY`，Z.AI 需要 `ZAI_API_KEY` 或 `ZHIPU_API_KEY`，Groq 需要 `GROQ_API_KEY`。缺少认证会导致生成器失败，而不是默默保留过时数据的理由。
