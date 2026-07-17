---
title: Kimi K3
description: Use Kimi K3 with its provider-recommended Kimi Code harness in Open Interpreter.
---

[Kimi K3](https://www.kimi.com/blog/kimi-k3) is Kimi's flagship model for
agentic coding and knowledge work. Open Interpreter reimplements the
provider-recommended [Kimi Code](https://www.kimi.com/coding/en) harness in
Rust, so K3 gets the request shape, tools, thinking history, and defaults it
expects inside a Codex-like interface.

## Start with a Kimi Code subscription

Install Open Interpreter, open your project, and start the terminal UI:

```bash
curl -fsSL https://www.openinterpreter.com/install | sh
cd my-project
i
```

On Windows, install with PowerShell, then run `i` in your project:

```powershell
irm https://www.openinterpreter.com/install.ps1 | iex
```

On first launch, choose **Kimi For Coding**, complete the Kimi sign-in in your
browser, then choose **Kimi K3**. In an existing session, open `/model` and make
the same selections. Start a new session when switching to K3 so it begins with
a fresh prompt cache and thinking history.

If you already have a compatible Kimi Code API key, you can start directly:

```bash
KIMI_API_KEY="..." interpreter \
  -c 'model_provider="kimi-for-coding"' \
  -m k3
```

Open Interpreter selects `kimi-code` automatically for Kimi providers. You do
not need to install or run the external Kimi Code CLI.

## Use a Moonshot Platform API key

Kimi K3 is also available as `kimi-k3` through the Moonshot Platform API:

```bash
MOONSHOT_API_KEY="..." interpreter \
  -c 'model_provider="moonshotai"' \
  -m kimi-k3
```

For a single non-interactive task:

```bash
MOONSHOT_API_KEY="..." interpreter exec \
  -c 'model_provider="moonshotai"' \
  -m kimi-k3 \
  "Review this repository and fix the highest-impact bug."
```

The Kimi Code harness is inferred for this provider as well. You can confirm or
change the active harness with `/harness`.

## Context and reasoning

Kimi currently recommends a fresh session when selecting K3 because switching
models invalidates the existing prompt cache. K3 uses maximum reasoning effort
at launch. Kimi Code subscriptions provide a 256K context window on Moderato
and up to 1M tokens on Allegretto and higher plans.

See Kimi's current [model configuration](https://www.kimi.com/code/docs/en/kimi-code/models.html)
for entitlement and reasoning details.

## Pricing

Kimi lists these Kimi Code subscription prices as of July 16, 2026:

| Plan | Monthly | Annual billing, per month | Kimi Code credits | K3 context |
| --- | ---: | ---: | ---: | --- |
| Moderato | $19 | $15 | 1× | 256K |
| Allegretto | $39 | $31 | 5× | Up to 1M |
| Allegro | $99 | $79 | 15× | Up to 1M |
| Vivace | $199 | $159 | 30× | Up to 1M |

Kimi's direct API pricing for K3 at launch is $0.30 per million cache-hit input
tokens, $3.00 per million cache-miss input tokens, and $15.00 per million
output tokens. Prices and plan entitlements can change; check Kimi's
[current membership pricing](https://www.kimi.com/help/membership/membership-pricing)
and [K3 launch post](https://www.kimi.com/blog/kimi-k3) before purchasing.

## Computer use

Kimi K3 can use Open Interpreter's bundled QA skill to operate and test
interfaces. Ask it to test a web app and it can use
[`agent-browser`](https://github.com/vercel-labs/agent-browser) in a real
browser. Ask it to test a native desktop app and it can use
[`trycua`](https://github.com/trycua/cua) for computer use.

For example:

```text
Run this app, test the sign-in flow like a user, and fix anything that breaks.
```

The normal sandbox and approval settings still apply. Keep the request
specific, and review actions that interact with accounts or external systems.

## ACP and Codex SDK

Use Kimi K3 from any supported editor or client by configuring it to launch
`interpreter acp`; see the [ACP guide](/docs/acp) and the current
[ACP client directory](https://agentclientprotocol.com/get-started/clients).

Existing Codex SDK integrations need only point their binary override at Open
Interpreter:

```diff
-const codex = new Codex();
+const codex = new Codex({ codexPathOverride: "interpreter" });
```

Set `model_provider` to `kimi-for-coding` with model `k3`, or to `moonshotai`
with model `kimi-k3`. See the [Codex SDK guide](/docs/sdk) for complete
TypeScript and Python examples.
