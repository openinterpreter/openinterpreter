---
title: Memories
description: Reuse personal preferences and durable context across sessions.
---

Memories are an optional feature for carrying useful personal context between
sessions. They are off by default.

```toml
[features]
memories = true

[memories]
use_memories = true
generate_memories = true
```

## Controls

| Key                                    | Purpose                                                     |
| -------------------------------------- | ----------------------------------------------------------- |
| `memories.use_memories`                | Inject relevant memories into future sessions.              |
| `memories.generate_memories`           | Generate new memory candidates from sessions.               |
| `memories.extract_model`               | Override the model used to extract raw memories.            |
| `memories.consolidation_model`         | Override the model used to consolidate memories.            |
| `memories.disable_on_external_context` | Skip memory generation for sessions using external context. |

## When to Use

Use memories for stable personal preferences, recurring project context, or
workflow habits that apply across many sessions. Do not use them for secrets,
temporary task state, or information that belongs in a repository file.

## Inspect

When enabled, use:

```text
/memories
```
