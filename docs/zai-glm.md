---
title: Z.AI, GLM, and ZCode
description: Use GLM models through Z.AI or Zhipu AI, with generic Chat or the native ZCode harness.
---

Open Interpreter includes providers for the global Z.AI platform, its GLM
Coding Plan, and the corresponding Zhipu AI services in China. It also includes
a native `zcode` harness for GLM coding workflows.

## Choose the Right Provider

| Account or service | Provider ID | Credential | Endpoint type |
| --- | --- | --- | --- |
| Z.AI pay-as-you-go API | `zai` | `ZAI_API_KEY` | General OpenAI-compatible Chat API |
| Z.AI GLM Coding Plan | `zai-coding-plan` | `ZAI_API_KEY` | Coding Plan OpenAI-compatible Chat API |
| Zhipu AI pay-as-you-go API | `zhipuai` | `ZHIPU_API_KEY` | General OpenAI-compatible Chat API |
| Zhipu AI Coding Plan | `zhipuai-coding-plan` | `ZHIPU_API_KEY` | Coding Plan OpenAI-compatible Chat API |

Z.AI documents separate general and Coding Plan endpoints. Use the Coding Plan
provider only for an eligible subscription; using the general endpoint does
not consume Coding Plan quota. Confirm that your client and use case comply
with Z.AI's current Coding Plan usage policy before relying on subscription
quota.

## Start With the Built-In Provider

Export the key for your account, start Open Interpreter, then use `/model` to
select the matching provider and a GLM model:

```bash
export ZAI_API_KEY="..."
interpreter
```

For a direct Z.AI Coding Plan launch:

```bash
ZAI_API_KEY="..." interpreter \
  -c 'model_provider="zai-coding-plan"' \
  -m glm-5.2
```

For the general Z.AI API, change the provider to `zai`. Users of the China
service should use `zhipuai` or `zhipuai-coding-plan` with `ZHIPU_API_KEY`.

The bundled providers use Z.AI's OpenAI-compatible Chat endpoint. With no
explicit harness, they use Open Interpreter's generic Chat agent surface.

## Use the ZCode Harness

`zcode` reproduces the ZCode-shaped system prompt, Messages request format,
tools, todos, plan controls, skills, session context, and subagent behavior in
Open Interpreter's native Rust runtime. It requires an Anthropic
Messages-compatible provider; selecting `zcode` on the built-in Chat provider
does not activate that Messages route.

Z.AI officially provides an Anthropic-compatible Coding Plan endpoint at
`https://api.z.ai/api/anthropic`. To use it, add a Messages provider to
`~/.openinterpreter/config.toml`:

```toml
model_provider = "zai-zcode"
model = "glm-5.2"
harness = "zcode"

[model_providers.zai-zcode]
name = "Z.AI ZCode"
base_url = "https://api.z.ai/api/anthropic"
env_key = "ZAI_API_KEY"
wire_api = "messages"
env_http_headers = { Authorization = "ZAI_AUTHORIZATION" }
```

Then expose the bearer header without writing the secret into the config file:

```bash
export ZAI_API_KEY="..."
export ZAI_AUTHORIZATION="Bearer $ZAI_API_KEY"
interpreter
```

Open Interpreter sends the ZCode Messages request to `/v1/messages`. The
additional authorization environment variable matches Z.AI's documented
bearer-token authentication while `ZAI_API_KEY` remains the provider's required
secret source.

See Z.AI's official [Coding Plan quickstart](https://docs.z.ai/devpack/quick-start)
and [tool endpoint guide](https://docs.z.ai/devpack/tool/others) for current
endpoint and plan-eligibility details.

## Choose a GLM Model

Use `/model` rather than maintaining a private list. The picker is generated
from maintained provider sources and includes current GLM model IDs for the
selected service. For example, the bundled Z.AI Coding Plan catalog currently
includes `glm-5.2`, `glm-5.1`, `glm-5-turbo`, and lower-cost models.

Z.AI may update server-side model mappings and plan eligibility independently
of Open Interpreter. Check its [model-switching guide](https://docs.z.ai/devpack/using5.1)
when a model is unavailable or consumes a different quota multiplier.

## Chat or ZCode?

| Goal | Configuration |
| --- | --- |
| Simplest setup with the built-in picker | `zai-coding-plan` or `zai`, generic Chat |
| Provider-recommended OpenAI-compatible integration | Built-in provider, `wire_api = "chat"` |
| ZCode-shaped coding-agent behavior | Custom Messages provider plus `harness = "zcode"` |

Do not set `wire_api = "messages"` on the OpenAI-compatible `/paas/v4`
endpoint, and do not point the Chat provider at `/api/anthropic`. The endpoint,
wire API, and harness must agree.

## Editors and SDKs

Both configurations can be used through `interpreter acp` in
[ACP-compatible editors](/docs/acp) and through Open Interpreter's
[Codex SDK compatibility](/docs/sdk). The selected provider and harness remain
part of the Open Interpreter configuration.

## Troubleshooting

- A 401 or 403 usually means the wrong key, endpoint, region, or authorization
  header is in use.
- If Coding Plan quota is not being used, confirm that the provider is
  `zai-coding-plan` or that the ZCode setup points to `/api/anthropic`.
- If `/harness` shows `zcode` but behavior looks generic, confirm the active
  provider has `wire_api = "messages"`.
- If a model is absent, open `/model` and choose one offered for that exact
  provider instead of copying a model ID from another Z.AI region or plan.
