<!-- README translation source: README.md sha256=d5db2b994d859b4186348f83bf35783a86b3f79e363d0c92100dbf06707ced16 -->

<h1 align="center">Open Interpreter</h1>

<p align="center">一款针对低成本模型优化的编程智能体。<a href="https://www.openinterpreter.com/blog/open-interpreter?utm_source=github&amp;utm_medium=referral&amp;utm_campaign=readme&amp;utm_content=hero_text"><strong>博客文章 ↗</strong></a></p>

<p align="center">
  <a href="README.md">English</a> • <a href="README_ES.md">Español</a> • <b>简体中文</b> • <a href="README_JA.md">日本語</a>
</p>

<p align="center">
  <a href="https://discord.gg/Hvz9Axh84z"><img alt="Discord" src="https://img.shields.io/discord/1146610656779440188?style=flat-square&label=Discord" /></a>
  <a href="https://www.openinterpreter.com/docs/terminal?utm_source=github&amp;utm_medium=referral&amp;utm_campaign=readme&amp;utm_content=docs_badge"><img alt="文档" src="https://img.shields.io/badge/Documentation-white?style=flat-square" /></a>
  <a href="LICENSE"><img alt="许可证" src="https://img.shields.io/badge/License-Apache--2.0-white?style=flat-square" /></a>
</p>

> [!NOTE]
> **今天：Kimi K3 正式上线。** 我们使用 Rust 重新实现了服务商推荐的
> [Kimi Code](https://www.kimi.com/coding/en) 智能体框架，
> 让你在熟悉的 Codex 风格界面中充分发挥 K3 的性能。
> [**Kimi 文档 →**](https://www.openinterpreter.com/docs/terminal/kimi-k3?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=kimi_k3_note)

<br>

<p align="center">
  <a href="https://www.openinterpreter.com/blog/open-interpreter?utm_source=github&amp;utm_medium=referral&amp;utm_campaign=readme&amp;utm_content=hero_image">
    <img alt="在终端中运行的 Open Interpreter" src="docs-site/assets/open-interpreter-terminal-hero.png" width="100%" />
  </a>
</p>

## 安装

macOS 和 Linux：

```bash
curl -fsSL https://www.openinterpreter.com/install | sh
```

Windows：

```powershell
irm https://www.openinterpreter.com/install.ps1 | iex
```

安装完成后，在终端中输入 `i` 或 `interpreter` 即可开始会话。

## 智能体框架模拟

Open Interpreter 是 OpenAI Codex 的一个分支，专注于模拟能够让低成本模型发挥最佳性能的智能体运行框架（harness）。

使用 `/harness` 切换当前框架：

```text
> /harness

native
claude-code
claude-code-bare
zcode
kimi-code
kimi-cli
qwen-code
deepseek-tui
swe-agent
minimal
```

更多信息请参阅[框架文档](https://www.openinterpreter.com/docs/terminal/harness?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=harness_docs)和[模型服务商配置指南](https://www.openinterpreter.com/docs/terminal/providers?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=provider_guides)。

## 兼容 ACP 和 Codex

Open Interpreter 可用于[兼容 ACP 的编辑器和客户端](https://agentclientprotocol.com/get-started/clients)。将客户端配置为启动 `interpreter acp`；具体示例请参阅 [ACP 指南](https://www.openinterpreter.com/docs/terminal/acp)。

已经在使用 OpenAI Codex SDK？保留原有 SDK，只需一行代码即可切换二进制文件：

```diff
-const codex = new Codex();
+const codex = new Codex({ codexPathOverride: "interpreter" });
```

Open Interpreter 使用与 Codex 相同的 `exec` 协议。请参阅 [SDK 指南](https://www.openinterpreter.com/docs/terminal/sdk)，并运行 `scripts/test-codex-sdk-compat.sh` 完成不依赖模型服务商的本地兼容性检查。

## 计算机操作

Open Interpreter 内置 QA 技能，让任何模型都能操作和测试界面。它可以通过 [agent-browser](https://github.com/vercel-labs/agent-browser) 在真实浏览器中操作 Web 应用，也可以通过 [trycua](https://github.com/trycua/cua) 操作和测试原生应用。

## 功能

- 在 macOS、Linux 和 Windows 上通过原生沙箱执行命令。
- 在 TUI 中使用 `/model` 切换模型服务商和模型。
- 通过 Chat Completions 运行任何兼容 OpenAI 的模型服务商：
  `interpreter --chat-completions` 或 `interpreter exec --chat-completions`。
- 使用 `/harness` 查看或切换 Rust 原生的模型框架。
- 通过内置 QA 技能测试 Web 应用和原生应用。
- 通过 `interpreter acp` 作为编辑器的 [Agent Client Protocol](https://agentclientprotocol.com/) 智能体运行。
- 将配置和会话状态保存在本地的 `~/.openinterpreter` 中。
- 支持 `exec`、MCP、技能、hooks、权限和 `AGENTS.md`。

## 文档

- [终端文档](https://www.openinterpreter.com/docs/terminal?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=terminal_docs)
- [快速开始](https://www.openinterpreter.com/docs/terminal/quickstart?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=quickstart)
- [安装指南](https://www.openinterpreter.com/docs/terminal/install?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=install_guide)
- [配置](https://www.openinterpreter.com/docs/terminal/config?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=configuration)
- [CLI 参考](https://www.openinterpreter.com/docs/terminal/cli-reference?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=cli_reference)
- [智能体框架](https://www.openinterpreter.com/docs/terminal/harness?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=harnesses)
- [模型服务商指南](https://www.openinterpreter.com/docs/terminal/providers?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=provider_guides)
  - [Kimi K3](https://www.openinterpreter.com/docs/terminal/kimi-k3?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=kimi_k3_docs)
  - [DeepSeek](https://www.openinterpreter.com/docs/terminal/deepseek?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=deepseek_docs)
  - [Z.AI、GLM 和 ZCode](https://www.openinterpreter.com/docs/terminal/zai-glm?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=zai_glm_docs)
- [Agent Client Protocol](https://www.openinterpreter.com/docs/terminal/acp)
- [Codex SDK](https://www.openinterpreter.com/docs/terminal/sdk)
- [沙箱与审批](https://www.openinterpreter.com/docs/terminal/sandbox?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=sandbox_approvals)
- [为发行版分支定制品牌](FORK_BRANDING.md)

模型服务商和模型列表由工具自动生成，而不是在 Rust 代码中手动维护。在 `codex-rs` 目录中运行 `python3 scripts/write_provider_catalog.py` 可刷新所有托管服务商；也可以多次使用 `--provider <provider-id>`，仅更新指定的服务商。实时模型源需要使用[服务商文档](https://www.openinterpreter.com/docs/terminal/providers?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=provider_catalog_generation)中说明的对应凭据。

> [!NOTE]
> 这是基于 Codex 构建的新版 Rust Open Interpreter。想找原来的 Python 项目？它现在由社区在 [endolith/open-interpreter](https://github.com/endolith/open-interpreter) 中继续维护。

## 许可证

Apache-2.0
