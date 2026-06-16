<h1 align="center">Open Interpreter</h1>

<p align="center">A coding agent for low-cost models.</p>

<p align="center">
  <a href="https://discord.gg/Hvz9Axh84z"><img alt="Discord" src="https://img.shields.io/discord/1146610656779440188?style=flat-square&label=Discord" /></a>
  <a href="https://www.openinterpreter.com/docs/terminal"><img alt="Documentation" src="https://img.shields.io/badge/Documentation-white?style=flat-square" /></a>
  <a href="LICENSE"><img alt="License" src="https://img.shields.io/badge/License-Apache--2.0-white?style=flat-square" /></a>
</p>

<p align="center">
  <a href="https://www.openinterpreter.com/docs/terminal">
    <img alt="A close-up of a laptop screen running a terminal agent" src="https://www.openinterpreter.com/blog/open-interpreter-1-0/blog-hero-1.jpg" width="720" />
  </a>
</p>

> [!NOTE]
> This is the new Rust version of Open Interpreter. Looking for the original Python project? It lives on as a community-maintained fork at [endolith/open-interpreter](https://github.com/endolith/open-interpreter).

### Installation

macOS and Linux:

```bash
curl -fsSL https://www.openinterpreter.com/install | sh
```

Windows:

```powershell
irm https://www.openinterpreter.com/install.ps1 | iex
```

Use the `www.openinterpreter.com` host in PowerShell. Windows PowerShell 5.1
does not follow the apex-domain `308 Permanent Redirect` from
`openinterpreter.com` before evaluating the installer script.

Then type `i` or `interpreter` in your terminal to start a session.

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

### Computer Use

Open Interpreter ships with a QA skill that lets any model operate and test interfaces. It can drive web apps in a real browser with [agent-browser](https://github.com/vercel-labs/agent-browser), or operate and test native apps with [trycua](https://github.com/trycua/cua).

### Features

- Runs commands inside native sandboxing on macOS, Linux, and Windows.
- Switches providers and models from the TUI with `/model`.
- Inspects or switches Rust-native model harnesses with `/harness`.
- Tests web and native apps through the built-in QA skill.
- Runs as an [Agent Client Protocol](https://agentclientprotocol.com/) agent for editors with `interpreter acp`.
- Keeps config and session state local under `~/.openinterpreter`.
- Supports `exec`, MCP, skills, hooks, permissions, and `AGENTS.md`.

### Documentation

- [Terminal docs](https://www.openinterpreter.com/docs/terminal)
- [Quickstart](https://www.openinterpreter.com/docs/terminal/quickstart)
- [Install guide](https://www.openinterpreter.com/docs/terminal/install)
- [Configuration](https://www.openinterpreter.com/docs/terminal/config)
- [CLI reference](https://www.openinterpreter.com/docs/terminal/cli-reference)
- [Harnesses](https://www.openinterpreter.com/docs/terminal/harness)
- [Model providers](https://www.openinterpreter.com/docs/terminal/providers)
- [Sandbox & approvals](https://www.openinterpreter.com/docs/terminal/sandbox)

### License

Apache-2.0
