---
title: 配置
description: 配置模型、提供商、审批、沙箱、功能、MCP 和配置文件。
---

Open Interpreter 从 TOML 文件读取持久化设置。用户级配置位于：

```text
~/.openinterpreter/config.toml
```

项目配置可以放在受信任的项目中：

```text
.openinterpreter/config.toml
```

命令行覆盖使用 `-c key=value`，仅对本次调用生效。

## 优先级

优先级更高的值会覆盖优先级更低的值：

1. 内置默认值
2. 系统或托管配置
3. 用户配置
4. 受信任的项目配置
5. 已选择的配置文件
6. 来自 `-c`、`--enable`、`--disable` 或专用标志的 CLI 覆盖

在 TUI 中使用 `/debug-config` 可查看实际生效的值以及它们的来源。

## 常用设置

```toml
model = "gpt-5.1-codex"
model_provider = "openai"

# "minimal" | "low" | "medium" | "high" | "xhigh"
model_reasoning_effort = "medium"

# "auto" | "concise" | "detailed" | "none"
model_reasoning_summary = "auto"

# "read-only" | "workspace-write" | "danger-full-access"
sandbox_mode = "workspace-write"

# "untrusted" | "on-request" | "never"
approval_policy = "on-request"

# "friendly" | "pragmatic" | "none"
personality = "pragmatic"

web_search = "cached"
log_dir = "~/.openinterpreter/log"
```

## 配置文件组

配置文件组是具名的设置集合：

```toml
[profiles.fast]
model = "gpt-5.1-codex-mini"
model_reasoning_effort = "low"

[profiles.review]
model = "gpt-5.1-codex"
model_reasoning_effort = "high"
sandbox_mode = "read-only"
```

使用方式：

```bash
interpreter --profile review
```

## CLI 覆盖

`-c` 接受类似 TOML 的值。请为字符串加引号，以防 shell 去掉它们：

```bash
interpreter -c model='"gpt-5.1-codex-mini"' -c approval_policy='"never"'
```

功能标志也有简写形式：

```bash
interpreter --enable hooks --disable memories
```

## 功能标志

可选行为位于 `[features]` 下。

```toml
[features]
hooks = true
multi_agent = true
shell_tool = true
shell_snapshot = true
unified_exec = true
memories = false
apps = false
plugins = false
undo = false
```

在 TUI 中使用 `/experimental` 可在可用时进行交互式切换。

## 模型提供商

内置提供商通过 `model_provider` 选择。可以在 `[model_providers.<id>]` 下添加自定义兼容 OpenAI 的提供商：

```toml
model_provider = "acme"
model = "acme-coder-large"

[model_providers.acme]
name = "Acme"
base_url = "https://api.acme.example/v1"
env_key = "ACME_API_KEY"
wire_api = "responses"
```

提供商凭证通常应来自环境变量或凭证存储，而不是内联令牌。

### app.nz

[app.nz](https://app.nz/) 是一个托管的 OpenAI 兼容网关。可以将它添加为自定义
chat-completions 提供商，并将 `model` 指向其 `app/auto` 自动路由器：

```toml
model_provider = "appnz"
model = "app/auto"

[model_providers.appnz]
name = "app.nz"
base_url = "https://app.nz/v1"
env_key = "APPNZ_API_KEY"
wire_api = "chat"
```

将 `APPNZ_API_KEY` 设置为你的 app.nz API 密钥。

### Cheaper Inference

[Cheaper Inference](https://cheaperinference.com/) 是一个托管的 OpenAI 兼容网关。
每个模型的价格比其所属实验室的标价低 15–60%。可以将它添加为自定义
chat-completions 提供商：

```toml
model_provider = "cheaperinference"
model = "gpt-5.4-mini"

[model_providers.cheaperinference]
name = "Cheaper Inference"
base_url = "https://api.cheaperinference.com/v1"
env_key = "CHEAPER_INFERENCE_API_KEY"
wire_api = "chat"
```

将 `CHEAPER_INFERENCE_API_KEY` 设置为你的 Cheaper Inference API 密钥。其他模型
ID（例如 `gpt-5.4` 或 `claude-sonnet-5`）请参阅
[模型列表](https://cheaperinference.com/#models)。

## Harness

Open Interpreter 增加了 `harness` 设置，用于兼容模式，使代理的交互界面类似其他编码 harness，同时仍在原生 Open Interpreter 运行时中运行。

```toml
harness = "kimi-code"
harness_guidance = true
```

支持的取值依实现而异，但当前代码库包括 native、Claude Code、DeepSeek TUI、当前 Kimi Code、旧版 Kimi CLI、Qwen Code、SWE-agent 以及 minimal harness 模式。仅当你刻意需要 harness 形态的提示/工具界面时才使用它。保持未设置则让 Open Interpreter 为所选提供商和模型自动选择推荐的 harness。

`harness_guidance` 让 Open Interpreter 在该 harness 模式允许的情况下加入一小段可靠性指导块。如果需要更严格的 harness 行为，请将其设为 `false`。

## MCP 服务器

MCP 服务器在 `[mcp_servers]` 下配置：

```toml
[mcp_servers.docs]
command = "npx"
args = ["-y", "@acme/docs-mcp"]
env = { ACME_TOKEN = "env:ACME_TOKEN" }
default_tools_approval_mode = "prompt"
```

支持流式 HTTP 服务器使用 `url`：

```toml
[mcp_servers.search]
url = "https://mcp.example.com"
bearer_token_env_var = "MCP_TOKEN"
```

请参阅 [MCP](/docs/mcp) 了解传输、OAuth 和每个工具的审批细节。

## Shell 环境

使用 `shell_environment_policy` 控制向生成的命令传递哪些环境变量：

```toml
[shell_environment_policy]
inherit = "all"
ignore_default_excludes = false
exclude = ["AWS_SECRET_ACCESS_KEY", "DATABASE_URL"]
set = { CI = "1" }
```

## 历史记录与记忆

会话历史本地存储。你可以关闭转录持久化：

```toml
[history]
persistence = "none"
```

记忆是另一个实验性功能：

```toml
[features]
memories = true

[memories]
use_memories = true
generate_memories = true
```

请参阅 [Memories](/docs/memories)。

## 配置模式

源码树中包含生成的 JSON Schema，路径为：

```text
codex-rs/core/config.schema.json
```

在维护共享配置时，可将其用于编辑器补全或 CI 校验。
