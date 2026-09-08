---
title: 模型
description: Open Interpreter 如何列出模型并展示模型能力。
---

使用 `/model` 来选择提供者、模型、harness（工具链）以及模型特定的控制。
页脚显示当前选中的模型。

## Shell 覆盖

```bash
interpreter -m gpt-5.1-codex "review this module"
interpreter --oss "use my local open source provider"
```

## 配置默认值

```toml
model_provider = "openai"
model = "gpt-5.1-codex"
model_reasoning_effort = "medium"
model_reasoning_summary = "auto"
model_verbosity = "medium"
```

## 易于验证的模型选择

以下是常见提供商的便捷起点。下面的 ID 要么继承自上游模型元数据，要么记录在提供商当前的官方来源中。实际可用性仍取决于账户、地区、套餐以及提供商返回的 `/models` 列表。使用 `/model` 可查看当前活动提供商实际提供的模型。

| 提供商 | 设置 | 文本模型 ID | Wire API |
| --- | --- | --- | --- |
| 上游继承的预设 | 仅在当前提供商提供这些 ID 时使用 | `gpt-6-astra`、`gpt-5.6-sol`、`gpt-5.6-terra`、`gpt-5.6-luna` | 提供商默认值 |
| Google AI Studio | 提供商 `google` 和 `GEMINI_API_KEY` | `gemini-3.8-flash`、`gemini-3.7-flash`、`gemini-3.6-flash`、`gemini-3.5-flash`、`gemini-3.5-flash-lite`、`gemini-3.1-flash-lite` | `chat` |
| Anthropic | 提供商 `anthropic` 和 `ANTHROPIC_API_KEY` | `claude-fable-5-1`、`claude-opus-5`、`claude-sonnet-5`、`claude-haiku-4-5-20251001` | 提供商默认值 |
| Z.AI | 提供商 `zai` 或 `zai-coding-plan` 和 `ZAI_API_KEY` | `glm-5.1`、`glm-5`、`glm-5-turbo`、`glm-4.7-flash` | `chat` |

第一行的 OpenAI 风格 ID 继承自上游的 `models-manager/models.json`，表示上游预设元数据，并不保证所有提供商都接受这些 ID。Google 当前的[官方 Gemini 模型列表](https://ai.google.dev/gemini-api/docs/models)包含所列的稳定 Gemini 3 Flash 文本 ID，并将 `gemini-3.8-flash` 列为其最智能的 Flash 模型。其[ OpenAI 兼容性指南](https://ai.google.dev/gemini-api/docs/openai)记录了兼容端点。对于 Z.AI，其 [GLM-5.1 指南](https://docs.z.ai/guides/llm/glm-5.1)和 [GLM-5-Turbo 指南](https://docs.z.ai/guides/llm/glm-5-turbo)使用上述 ID；请使用所选服务返回的准确 ID，不要猜测别名。

## 模型元数据的来源

Open Interpreter 并未维护一份手写的 Rust 列表来列出所有模型。元数据分层如下：

| Source | Role |
| --- | --- |
| Provider `/models` endpoint | 在端点可用时，获取活动提供者的实时模型 ID。 |
| `model-provider-info/provider_catalog.json` | 由 `models.dev` 生成并结合已配置的实时提供者模型来源的捆绑 provider/model 种子数据。 |
| `codex-api/model_compatibility_catalog.json` | 兼容性元数据，如支持的参数、搜索支持、推理等级和输入模式。 |
| `models-manager/models.json` | 管理器使用的 OpenAI 风格模型预设元数据。 |
| Config `model_catalog` | 可选的用户提供的静态目录，用于特定提供者/会话。 |

模型管理器会向活动提供者请求模型列表，然后在能够通过 Anthropic 身份、基础 URL、提供者名称或认证环境变量识别提供者时使用捆绑数据。这使得代理配置在明确指向已知提供者时仍能继承有用的元数据。

## 功能元数据

模型元数据可以控制：

- 选择器可见性；
- 显示名称和描述；
- 上下文窗口；
- 输入模式，如文本和图像；
- 模型是否受 API 支持；
- 支持的请求参数；
- 推理控制形态；
- 网页/搜索支持；
- 并行工具调用支持。

在 UI 中，推理并不是单一的布尔值。协议中有以下控制形态：

| Control | Meaning |
| --- | --- |
| `none` | 无已知推理控制。 |
| `fixed` | 模型会推理，但 UI 不应暴露控制。 |
| `effort` | OpenAI 风格的努力控制。 |
| `thinking_toggle` | 布尔型思考开/关控制。 |
| `thinking_budget` | 令牌预算思考控制。 |

## 推理努力

当模型暴露努力控制时，Open Interpreter 使用以下取值：

| 值 | 用途 |
| --- | --- |
| `minimal` | 快速、简单的编辑。 |
| `low` | 常规实现。 |
| `medium` | 默认的平衡工作。 |
| `high` | 硬核调试、重构、审查。 |
| `xhigh` | 模型特定的额外推理。 |

不同的 harness 可能会将这些取值映射到提供者特定的字段。例如，`kimi-cli` 将 `minimal` 和 `low` 映射为低推理，`medium` 映射为中等，`high` 或 `xhigh` 映射为高。

不受支持的模型会根据提供者行为隐藏、忽略或拒绝推理控制。

## 输入模式

标准的输入模式标签如下：

| 值 | 含义 |
| --- | --- |
| `text` | 正常的用户回合和工具负载。 |
| `image` | 通过 `-i` 等命令附加的图像。 |

附加图像：

```bash
interpreter -i screenshot.png "what is wrong here?"
```

省略模式元数据的旧式负载为兼容性保守地默认支持文本和图像，但生成的提供者条目应在已知时注明真实的模式。

## 本地模型

Open Interpreter 有两个内置的本地 OSS 提供者：

| 提供商 | 默认基础 URL | 覆盖方式 |
| --- | --- | --- |
| `ollama` | `http://localhost:11434/v1` | `CODEX_OSS_PORT` 或 `CODEX_OSS_BASE_URL` |
| `lmstudio` | `http://localhost:1234/v1` | `CODEX_OSS_PORT` 或 `CODEX_OSS_BASE_URL` |

在启动 Open Interpreter 之前先启动本地服务，然后直接指定提供者：

```bash
interpreter --oss --local-provider ollama
interpreter --oss --local-provider lmstudio
```

不带 `--local-provider` 的 `--oss` 会使用你已保存的 `oss_provider`，或弹出选择器显示每个默认本地端点是否有响应。

若服务器位于其他主机或端口，需在启动 Open Interpreter 前设置完整的兼容 OpenAI 的 `/v1` 基础 URL：

```bash
CODEX_OSS_BASE_URL=http://192.168.1.20:1234/v1 \
  interpreter --oss --local-provider lmstudio -m qwen/qwen3-coder-next
```

远程 Ollama 服务器请使用 `--local-provider ollama`。不要仅仅为了更改任一内置本地提供者的地址而创建单独的 `model_providers` 条目；`CODEX_OSS_BASE_URL` 才是受支持的覆盖方式。

### 本地模型的工具调用

Agent 提示词除了用户输入外，还包含指令和工具 schema。请确保本地服务器实际启用的上下文窗口足以容纳完整请求。例如，即使模型训练时支持 32K 上下文，Ollama 仍可能以较小的服务器默认值运行；可在启动服务器前提高该值：

```bash
OLLAMA_CONTEXT_LENGTH=32768 ollama serve
```

如果服务器截断了请求开头，模型可能会猜测文件内容、把工具调用作为普通文本输出，或者要求用户代为运行命令，而不是实际调用工具。这些现象表示模型或服务器存在兼容性问题，并不能证明请求的文件系统操作已经发生。

`qwen-code` harness 使用 Chat Completions 传输。通过 Ollama 使用 Qwen 模型时，请显式选择该传输，使自动推断的 Qwen harness 能使用其原生请求格式：

```bash
interpreter exec --oss --local-provider ollama --chat-completions \
  -m qwen2.5-coder:7b "inspect this project"
```

在自动化任务中，还应在命令退出后直接检查预期文件或其他副作用。请参阅[非交互模式](/docs/exec#完成状态与退出码)。

### 模型元数据警告

`Model metadata for ... not found` 表示本地服务器返回的模型 ID 未在 Open Interpreter 的兼容性目录中。它本身并不意味着服务器连接失败。请确认服务器暴露的准确 ID，使用相同的值通过 `-m` 传入，并更新 Open Interpreter 以获得最新目录。Open Interpreter 可以继续使用回退元数据，但某些模型特定的控制或行为可能不可用。

## 提供者系列与 Harness 默认值

某些模型系列在未显式指定 harness 时会使用默认 harness：

| 模型/提供商系列 | 默认 harness |
| --- | --- |
| Claude/Anthropic/Messages | `claude-code` |
| Kimi/Moonshot | `kimi-code` |
| Qwen/QwQ/DashScope | `qwen-code` |
| DeepSeek | `claude-code-bare` |

请参阅 [Harness](/docs/harness) 获取 wire‑api 兼容性矩阵，及 [Model providers](/docs/providers) 获取针对各提供者的设置指南。
