---
title: Execution policy
description: Rules that classify commands as safe, risky, or blocked.
---

Execution policy is the rule layer underneath the sandbox. It looks at
each command before it runs and labels it.

| Label   | What it means                                     |
| ------- | ------------------------------------------------- |
| `safe`  | Routine, low-risk. Goes through without prompts.  |
| `unsafe`| Could change state. Subject to approval.          |
| `forbid`| Always blocked.                                   |

Open Interpreter ships with a sensible default policy. Most users never
edit it. You only need this page if you want tighter control or you are
running on a shared system.

## Where it lives

The policy is loaded from `config.toml`. Each rule is a pattern that
matches a command and an action.

```toml
[[execpolicy.rules]]
match = "ls"
action = "safe"

[[execpolicy.rules]]
match = "rm *"
action = "unsafe"

[[execpolicy.rules]]
match = "rm -rf /"
action = "forbid"
```

Rules are evaluated top to bottom. The first match wins.

## How it interacts with approvals

The execution policy is the agent's first filter. After the policy
labels a command:

1. `forbid` blocks the command outright.
2. `safe` runs without prompting.
3. `unsafe` defers to your approval mode (see [Sandbox & approvals](/docs/sandbox)).

So a `safe` rule can trim approvals from your day, and a `forbid` rule
guarantees a command never runs even if you accidentally press `y`.

## A common pattern

Make the lint and test commands you trust completely free of prompts:

```toml
[[execpolicy.rules]]
match = "pnpm test*"
action = "safe"

[[execpolicy.rules]]
match = "pnpm lint*"
action = "safe"
```

Force confirmation for anything destructive:

```toml
[[execpolicy.rules]]
match = "git push --force*"
action = "unsafe"

[[execpolicy.rules]]
match = "drop database*"
action = "forbid"
```

<Tip>
Patterns use shell-style globs. Test rules with
`interpreter execpolicy check '<command>'` before relying on them in
automation.
</Tip>
