---
title: DeepSeek
description: 使用默认的 Claude Code 裸装置或可选的 DeepSeek TUI 装置来使用 DeepSeek 模型。
---

Open Interpreter 通过内置的 `deepseek` 提供程序直接连接到
[DeepSeek API](https://api-docs.deepseek.com/)。它使用 DeepSeek 的兼容 OpenAI 的 Chat 端点，并从 `DEEPSEEK_API_KEY` 中读取您的密钥。

## 开始使用 DeepSeek

在 DeepSeek 平台创建密钥，导出它，然后启动 Open Interpreter：

```bash
export DEEPSEEK_API_KEY="..."
interpreter
```

打开 `/model`，选择 **DeepSeek**，然后选择模型。直接启动方式：

```bash
DEEPSEEK_API_KEY="..." interpreter \
  -c 'model_provider="deepseek"' \
  -m deepseek-v4-pro
```

执行单个非交互式任务：

```bash
DEEPSEEK_API_KEY="..." interpreter exec \
  -c 'model_provider="deepseek"' \
  -m deepseek-v4-pro \
  "Review this repository and fix the highest-impact bug."
```

## 选择模型

`/model` 选择器由维护的提供程序目录和提供程序当前的模型数据生成。请将其作为您已安装版本中可用模型的唯一可信来源。

DeepSeek 当前的 API 模型 ID 为 `deepseek-flash` 和 `deepseek-v4-pro`。在需要更高容量时使用 Pro，在成本或并发更重要时使用 Flash。旧的 `deepseek-v4-flash` 仅作为兼容别名指向现行 Flash 模型；`deepseek-chat` 和 `deepseek-reasoner` 原定于 2026 年 7 月 24 日停用，新配置不应使用这些旧 ID。请查看 DeepSeek 的[API 更新](https://api-docs.deepseek.com/updates/)以获取当前可用性，并在提交工作负载前查阅其[定价页面](https://api-docs.deepseek.com/quick_start/pricing)。

## 装置行为

当您未设置装置时，DeepSeek 模型会自动使用 `claude-code-bare`。Open Interpreter 在 DeepSeek 的 Chat Completions 端点之上提供了更小的 Claude Code 形态的代理界面；它不运行外部 CLI。

要检查或更改当前装置，请运行 `/harness`。当您特别想要 DeepSeek 的 TUI/CodeWhale 形态提示和工具时，可使用可选的 `deepseek-tui` 模式：

```toml
model_provider = "deepseek"
model = "deepseek-v4-pro"
harness = "deepseek-tui"
```

删除显式的 `harness` 行即可恢复推荐的自动默认装置。

## 配置

内置提供程序相当于以下连接方式：

```toml
model_provider = "deepseek"
model = "deepseek-v4-pro"

[model_providers.deepseek]
name = "DeepSeek"
base_url = "https://api.deepseek.com"
env_key = "DEEPSEEK_API_KEY"
wire_api = "chat"
```

通常您不需要复制该提供程序块。仅在需要了解代理或自定义部署时才有用。如果您更改了端点，请保持 `wire_api = "chat"` 以使用 DeepSeek 的兼容 OpenAI 的 Chat API。

## 编辑器和 SDK

相同的提供程序配置可通过 `interpreter acp` 在
[ACP 兼容编辑器](/docs/acp) 中使用，也可通过 Open Interpreter 的
[Codex SDK 兼容性](/docs/sdk) 使用。

## 故障排除

- 401 或 403 通常表示 `DEEPSEEK_API_KEY` 缺失、已过期或属于不同的端点。
- 如果旧的模型 ID 停止工作，请打开 `/model` 并选择当前的 V4 模型，而不是硬编码替代模型。
- 如果代理界面出现异常，运行 `/harness` 并删除 `~/.openinterpreter/config.toml` 中的任何旧的显式 `harness` 值。
- 如果代理暴露了不同的协议，请匹配其 `wire_api`；不要假设每个 DeepSeek 兼容的端点都使用相同的传输方式。
