---
title: Z.AI、GLM 与 ZCode
description: 通过 Z.AI 或 Zhipu AI 使用 GLM 模型，可使用通用 Chat 或原生 ZCode harness。
---

Open Interpreter 包含面向全球 Z.AI 平台、其 GLM Coding Plan 以及中国对应 Zhipu AI 服务的提供商。它还提供原生的 `zcode` harness，用于 GLM 编码工作流。

## 选择合适的提供商

| 账户或服务 | 提供商 ID | 凭证 | 端点类型 |
| --- | --- | --- | --- |
| Z.AI 按量付费 API | `zai` | `ZAI_API_KEY` | 通用 OpenAI 兼容 Chat API |
| Z.AI GLM Coding Plan | `zai-coding-plan` | `ZAI_API_KEY` | Coding Plan 的 OpenAI 兼容 Chat API |
| Zhipu AI 按量付费 API | `zhipuai` | `ZHIPU_API_KEY` | 通用 OpenAI 兼容 Chat API |
| Zhipu AI Coding Plan | `zhipuai-coding-plan` | `ZHIPU_API_KEY` | Coding Plan 的 OpenAI 兼容 Chat API |

Z.AI 文档将通用端点和 Coding Plan 端点分开。仅在具备相应订阅资格时使用 Coding Plan 提供商；使用通用端点不会消耗 Coding Plan 配额。请在依赖订阅配额前确认您的客户端和使用场景符合 Z.AI 当前的 Coding Plan 使用政策。

## 从内置提供商开始

导出账户密钥，启动 Open Interpreter，然后使用 `/model` 选择对应的提供商和 GLM 模型：

```bash
export ZAI_API_KEY="..."
interpreter
```

直接使用 Z.AI Coding Plan 启动：

```bash
ZAI_API_KEY="..." interpreter \
  -c 'model_provider="zai-coding-plan"' \
  -m glm-5.1
```

若使用通用 Z.AI API，将提供商改为 `zai`。使用中国区服务的用户应使用 `zhipuai` 或 `zhipuai-coding-plan` 并配合 `ZHIPU_API_KEY`。

这些捆绑的提供商使用 Z.AI 的 OpenAI 兼容 Chat 端点。若未显式指定 harness，则使用 Open Interpreter 的通用 Chat 代理界面。

## 使用 ZCode Harness

`zcode` 在 Open Interpreter 的原生 Rust 运行时中复现了 ZCode 形态的系统提示、Messages 请求格式、工具、待办事项、计划控制、技能、会话上下文以及子代理行为。它需要一个 Anthropic Messages 兼容的提供商；在内置 Chat 提供商上选择 `zcode` 并不会激活该 Messages 路径。

Z.AI 官方提供了 Anthropic 兼容的 Coding Plan 端点，地址为 `https://api.z.ai/api/anthropic`。要使用它，请在 `~/.openinterpreter/config.toml` 中添加一个 Messages 提供商：

```toml
model_provider = "zai-zcode"
model = "glm-5.1"
harness = "zcode"

[model_providers.zai-zcode]
name = "Z.AI ZCode"
base_url = "https://api.z.ai/api/anthropic"
env_key = "ZAI_API_KEY"
wire_api = "messages"
env_http_headers = { Authorization = "ZAI_AUTHORIZATION" }
```

随后在不将密钥写入配置文件的情况下暴露 bearer 头部：

```bash
export ZAI_API_KEY="..."
export ZAI_AUTHORIZATION="Bearer $ZAI_API_KEY"
interpreter
```

Open Interpreter 会将 ZCode Messages 请求发送至 `/v1/messages`。额外的授权环境变量符合 Z.AI 文档中所述的 bearer‑token 认证，而 `ZAI_API_KEY` 仍是提供商必需的密钥来源。

请参阅 Z.AI 官方的[编码计划快速入门](https://docs.z.ai/devpack/quick-start)和[工具端点指南](https://docs.z.ai/devpack/tool/others)了解最新的端点和计划资格细节。

## 选择 GLM 模型

使用 `/model` 而不是维护私有列表。该选择器由维护的提供商来源生成，包含所选服务当前可用的 GLM 模型 ID。提供商文档中可作为起点的模型是 `glm-5.1`、`glm-5`、`glm-5-turbo` 和免费轻量级的 `glm-4.7-flash`；请使用所选服务返回的准确 ID，不要猜测版本别名。

Z.AI 可能会在服务器端独立于 Open Interpreter 更新模型映射和计划资格。模型不可用或使用了不同配额倍率时，请查阅其[模型切换指南](https://docs.z.ai/devpack/using5.1)。

## Chat 还是 ZCode？

| 目标 | 配置 |
| --- | --- |
| 使用内置选择器的最简配置 | `zai-coding-plan` 或 `zai`，通用 Chat |
| 提供商推荐的 OpenAI 兼容集成 | 内置提供商，`wire_api = "chat"` |
| ZCode 形态的编码代理行为 | 自定义 Messages 提供商加 `harness = "zcode"` |

不要在 OpenAI 兼容的 `/paas/v4` 端点上设置 `wire_api = "messages"`，也不要将 Chat 提供商指向 `/api/anthropic`。端点、wire API 与 harness 必须保持一致。

## 编辑器和 SDK

两种配置均可通过 `interpreter acp` 在[兼容 ACP 的编辑器](/docs/acp)中使用，也可通过 Open Interpreter 的[Codex SDK 兼容性](/docs/sdk)使用。所选提供商和 harness 仍是 Open Interpreter 配置的一部分。

## 故障排除

- 401 或 403 通常表示使用了错误的密钥、端点、地区或授权头部。
- 若未使用 Coding Plan 配额，请确认提供商为 `zai-coding-plan`，或确认 ZCode 设置指向 `/api/anthropic`。
- 若 `/harness` 显示 `zcode` 但行为仍然通用，请确认当前提供商的 `wire_api = "messages"` 已生效。
- 若缺少某个模型，打开 `/model` 并选择该提供商专属提供的模型，而不要从其他 Z.AI 区域或计划复制模型 ID。
