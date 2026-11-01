---
title: Contributing
description: How to report issues, propose changes, and work on Open Interpreter.
---

Open Interpreter is a fork of Codex with a narrower product focus. We accept
contributions that affect what this fork owns: Open Interpreter's low-memory
multi-tab runtime, model/provider harness behavior, Open Interpreter-specific
TUI and onboarding changes, installer/update behavior, and product
documentation.

For generic Codex CLI behavior that is not specific to Open Interpreter, please
contribute upstream to [OpenAI Codex](https://github.com/openai/codex). Keeping
general-purpose fixes upstream helps both projects and reduces long-term fork
drift.

## Good First Step

Before opening a pull request, open or join an issue so the behavior and scope
are clear. That is especially important for changes to:

- low-memory multi-tab behavior and shared-runtime work;
- harness selection, harness compatibility, and provider-specific coding-agent
  behavior;
- Open Interpreter-specific TUI, onboarding, model picker, or status UI;
- installer, standalone layout, and update behavior;
- provider/model catalog generation for Open Interpreter-supported providers;
- Open Interpreter docs, examples, and migration guidance.

Changes that are primarily upstream Codex concerns should start in the upstream
Codex repository instead. Examples include generic sandbox internals, generic
MCP protocol behavior, generic app-server protocol changes, or broad CLI
features that are not tied to Open Interpreter's product direction.

## Pull Requests

Keep changes focused and easy to review. If a change affects user behavior,
update the relevant docs in this folder and the CLI help where appropriate.

For code changes:

- run the formatter for the area you changed;
- run the narrowest meaningful test target first;
- include regression tests for behavior changes when practical;
- avoid unrelated refactors in the same pull request.

## Model And Provider Metadata

Provider/model membership is generated. Do not hand-patch model lists in Rust
as a product fix.

When model catalog behavior changes, update the generator inputs or overrides
and regenerate the catalog artifacts. The main generators live under:

```text
codex-rs/scripts/write_provider_catalog.py
codex-rs/scripts/write_model_compatibility_catalog.py
```

`write_provider_catalog.py --provider <id>` refreshes only the named provider
in the existing generated artifact. Repeat the option to refresh a related set
without requiring credentials for unrelated live provider sources.

Update [Models](/docs/models), [Providers](/docs/providers), or
[Config reference](/docs/config-reference) when the user-facing setup changes.

## Security

Do not report vulnerabilities in public issue threads. Follow the repository's
[security policy](https://github.com/openinterpreter/openinterpreter/blob/main/SECURITY.md) instead.

## Community values

- Be kind and inclusive, and treat other contributors with respect.
- Assume good intent when written communication is ambiguous.
- Share reproduction details and technical findings so the whole community can
  benefit.
