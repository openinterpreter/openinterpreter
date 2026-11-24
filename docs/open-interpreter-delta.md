---
title: Open Interpreter maintained Codex delta
description: The explicit product and compatibility differences preserved while tracking upstream Codex.
---

# Open Interpreter's maintained Codex delta

Open Interpreter tracks stable releases of the upstream Codex CLI. We keep a
small, explicit product delta so upstream updates can be reviewed without
accidentally dropping compatibility behavior.

The maintained delta is:

- Open Interpreter product identity, packaging, installers, updates, and
  release workflow.
- Provider-neutral model configuration and the hosted-provider catalog.
- First-class OpenAI-compatible Chat Completions transport, selectable with
  `--chat-completions` for one invocation or `wire_api = "chat"` in a provider
  configuration.
- Anthropic Messages-compatible transport for providers that expose that API.
- Native provider harnesses and agent surfaces that remain provider-neutral.
- Open Interpreter desktop and app-server integration points.

Internal Codex-compatible crate, protocol, configuration, and environment
names are retained when interoperability requires them. User-facing help,
errors, completions, installers, and update surfaces must use Open Interpreter
identity unless they are explicitly describing upstream compatibility.

Every stable upstream reconciliation must review this list, test each affected
bucket, and record any addition or removal in the release pull request.

## Upstream maintenance checklist

- [ ] Preserve `--chat-completions` on both `interpreter` and
      `interpreter exec`; confirm it selects `wire_api = "chat"` without requiring
      an undocumented configuration override.
- [ ] Exercise a Chat Completions provider with text, streaming, tool calls,
      tool results, and usage metadata.
- [ ] Run `interpreter --help`, `interpreter exec --help`, and
      `interpreter --version`; none may present Codex as the installed product.
- [ ] Generate completions for every supported shell and confirm the command
      name is `interpreter` and all public flags, including `--chat-completions`,
      are present.
- [ ] Sweep visible help, warnings, errors, onboarding, status, resume, update,
      installer, and release output for inappropriate `Codex`, `codex`, or
      `CODEX_` product names. Classify retained matches as upstream attribution,
      model names, protocol/crate identifiers, or documented compatibility aliases.
- [ ] Run the focused product identity, CLI parsing, Chat wire compatibility,
      installer, and release workflow tests before publishing.
