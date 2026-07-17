<h1 align="center">Open Interpreter</h1>

<p align="center">A coding agent optimized for low-cost models. <a href="https://www.openinterpreter.com/blog/open-interpreter?utm_source=github&amp;utm_medium=referral&amp;utm_campaign=readme&amp;utm_content=hero_text"><strong>Blog post ↗</strong></a></p>

<p align="center">
  <a href="https://discord.gg/Hvz9Axh84z"><img alt="Discord" src="https://img.shields.io/discord/1146610656779440188?style=flat-square&label=Discord" /></a>
  <a href="https://www.openinterpreter.com/docs/terminal?utm_source=github&amp;utm_medium=referral&amp;utm_campaign=readme&amp;utm_content=docs_badge"><img alt="Documentation" src="https://img.shields.io/badge/Documentation-white?style=flat-square" /></a>
  <a href="LICENSE"><img alt="License" src="https://img.shields.io/badge/License-Apache--2.0-white?style=flat-square" /></a>
</p>

> [!NOTE]
> **Today: Kimi K3 is here.** We have reimplemented the provider-recommended
> [Kimi Code](https://www.kimi.com/coding/en) harness in Rust, giving you
> maximum K3 performance with a Codex-like interface.
> [**Kimi Docs →**](https://www.openinterpreter.com/docs/terminal/kimi-k3?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=kimi_k3_note)

<br>

<p align="center">
  <a href="https://www.openinterpreter.com/blog/open-interpreter?utm_source=github&amp;utm_medium=referral&amp;utm_campaign=readme&amp;utm_content=hero_image">
    <img alt="Open Interpreter running in a terminal" src="https://openinterpreter.com/blog/open-interpreter/blog-hero-1.jpg" width="100%" />
  </a>
</p>

### Installation

macOS and Linux:

```bash
curl -fsSL https://www.openinterpreter.com/install | sh
```

Windows:

```powershell
irm https://www.openinterpreter.com/install.ps1 | iex
```

Then type `i` or `interpreter` in your terminal to start a session.

### Harness Emulation

Open Interpreter is a fork of OpenAI's Codex, with a focus on emulating the agent harness that gets the best performance out of low-cost models.

Use `/harness` to switch the active harness:

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

Kimi and Moonshot models use the current `kimi-code` harness by default.
`kimi-cli` remains available for compatibility with the retired Python CLI
profile.

Read more in the [harness docs](https://www.openinterpreter.com/docs/terminal/harness?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=harness_docs) and [model provider docs](https://www.openinterpreter.com/docs/terminal/providers?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=model_provider_docs).

### ACP compatible, Codex compatible

Open Interpreter works in [ACP-compatible editors and clients](https://agentclientprotocol.com/get-started/clients). Configure the client to launch `interpreter acp`; see the [ACP guide](https://www.openinterpreter.com/docs/terminal/acp) for examples.

Already building with OpenAI's Codex SDK? Keep the SDK and make a one-line
binary override:

```diff
-const codex = new Codex();
+const codex = new Codex({ codexPathOverride: "interpreter" });
```

Open Interpreter speaks the same Codex exec protocol. See the [SDK guide](https://www.openinterpreter.com/docs/terminal/sdk) and run `scripts/test-codex-sdk-compat.sh` for a local, provider-free compatibility check.

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

- [Terminal docs](https://www.openinterpreter.com/docs/terminal?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=terminal_docs)
- [Quickstart](https://www.openinterpreter.com/docs/terminal/quickstart?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=quickstart)
- [Install guide](https://www.openinterpreter.com/docs/terminal/install?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=install_guide)
- [Configuration](https://www.openinterpreter.com/docs/terminal/config?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=configuration)
- [CLI reference](https://www.openinterpreter.com/docs/terminal/cli-reference?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=cli_reference)
- [Harnesses](https://www.openinterpreter.com/docs/terminal/harness?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=harnesses)
- [Kimi K3](https://www.openinterpreter.com/docs/terminal/kimi-k3?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=kimi_k3_docs)
- [Model providers](https://www.openinterpreter.com/docs/terminal/providers?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=model_providers)
- [Agent Client Protocol](https://www.openinterpreter.com/docs/terminal/acp)
- [Codex SDK](https://www.openinterpreter.com/docs/terminal/sdk)
- [Sandbox & approvals](https://www.openinterpreter.com/docs/terminal/sandbox?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=sandbox_approvals)

Provider and model membership is generated, not maintained as Rust lists. From
`codex-rs`, refresh all hosted providers with
`python3 scripts/write_provider_catalog.py`, or repeat
`--provider <provider-id>` to update only selected provider entries. Live model
sources require the provider credentials documented in the
[provider docs](https://www.openinterpreter.com/docs/terminal/providers?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=provider_catalog_generation).


> [!NOTE]
> This is the new Rust version of Open Interpreter, based on Codex. Looking for the original Python project? It lives on as a community-maintained fork at [endolith/open-interpreter](https://github.com/endolith/open-interpreter).

### License

Apache-2.0
