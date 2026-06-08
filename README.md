<h1 align="center">Open Interpreter</h1>

<p align="center">A coding agent for low-cost models.</p>

<p align="center">
  <a href="https://discord.gg/Hvz9Axh84z"><img alt="Discord" src="https://img.shields.io/discord/1146610656779440188?style=flat-square&label=Discord" /></a>
  <a href="https://www.openinterpreter.com/docs/terminal"><img alt="Documentation" src="https://img.shields.io/badge/Documentation-white?style=flat-square" /></a>
  <a href="LICENSE"><img alt="License" src="https://img.shields.io/badge/License-Apache--2.0-white?style=flat-square" /></a>
</p>

<p align="center">
  <a href="https://www.openinterpreter.com"><img alt="English" src="https://img.shields.io/badge/English-white?style=flat-square" /></a>
  <a href="https://www.openinterpreter.com/es"><img alt="Español" src="https://img.shields.io/badge/Espa%C3%B1ol-white?style=flat-square" /></a>
  <a href="https://www.openinterpreter.com/fr"><img alt="Français" src="https://img.shields.io/badge/Fran%C3%A7ais-white?style=flat-square" /></a>
  <a href="https://www.openinterpreter.com/de"><img alt="Deutsch" src="https://img.shields.io/badge/Deutsch-white?style=flat-square" /></a>
  <a href="https://www.openinterpreter.com/pt"><img alt="Português" src="https://img.shields.io/badge/Portugu%C3%AAs-white?style=flat-square" /></a>
  <a href="https://www.openinterpreter.com/zh"><img alt="中文" src="https://img.shields.io/badge/%E4%B8%AD%E6%96%87-white?style=flat-square" /></a>
  <a href="https://www.openinterpreter.com/ja"><img alt="日本語" src="https://img.shields.io/badge/%E6%97%A5%E6%9C%AC%E8%AA%9E-white?style=flat-square" /></a>
</p>

[![A close-up of a laptop screen running a terminal agent](https://www.openinterpreter.com/blog/open-interpreter-1-0/blog-hero-1.jpg)](https://www.openinterpreter.com/docs/terminal)

<br>

> [!NOTE]
> **JUNE 2026: THE NEW ● OPEN INTERPRETER is in pre-release.** Check out the [blog post](https://www.openinterpreter.com/blog/open-interpreter-1-0) and let us know what you think on [Discord](https://discord.gg/Hvz9Axh84z).

<br>

### Installation

macOS and Linux:

```bash
curl -fsSL https://openinterpreter.com/install | sh
```

Windows:

```powershell
irm https://openinterpreter.com/install.ps1 | iex
```

Then type `i` or `interpreter` in your terminal to start a session.

<br>

### Harness Emulation

Open Interpreter is a fork of OpenAI's Codex, with a focus on emulating the agent harness that gets the best performance out of low-cost models.

Use `/harness` to switch the active harness:

```text
> /harness

native
claude-code
claude-code-bare
kimi-cli
qwen-code
deepseek-tui
swe-agent
minimal
```

Read more in the [harness docs](https://www.openinterpreter.com/docs/terminal/harness) and [model provider docs](https://www.openinterpreter.com/docs/terminal/providers).

<br>

### Computer Use

Open Interpreter ships with a QA skill that lets any model operate and test interfaces. It can drive web apps in a real browser with [agent-browser](https://github.com/vercel-labs/agent-browser), or operate and test native apps with [trycua](https://github.com/trycua/cua).

<br>

### Features

- Runs commands inside native sandboxing on macOS, Linux, and Windows.
- Switches providers and models from the TUI with `/model`.
- Inspects or switches Rust-native model harnesses with `/harness`.
- Tests web and native apps through the built-in QA skill.
- Shares one local runtime across terminal tabs instead of starting a full agent runtime for every session.
- Keeps config and session state local under `~/.openinterpreter`.
- Supports `exec`, MCP, skills, hooks, permissions, and `AGENTS.md`.

<br>

### Documentation

- [Terminal docs](https://www.openinterpreter.com/docs/terminal)
- [Quickstart](https://www.openinterpreter.com/docs/terminal/quickstart)
- [Install guide](https://www.openinterpreter.com/docs/terminal/install)
- [Configuration](https://www.openinterpreter.com/docs/terminal/config)
- [CLI reference](https://www.openinterpreter.com/docs/terminal/cli-reference)
- [Harnesses](https://www.openinterpreter.com/docs/terminal/harness)
- [Model providers](https://www.openinterpreter.com/docs/terminal/providers)
- [Sandbox & approvals](https://www.openinterpreter.com/docs/terminal/sandbox)

<br>

### License

Apache-2.0
