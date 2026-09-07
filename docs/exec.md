---
title: Non-Interactive Mode
description: Run Open Interpreter from scripts, CI, and pipelines with interpreter exec.
---

Use `interpreter exec` when you want one task to run to completion without the
full-screen TUI.

```bash
interpreter exec "summarize the changes in the last commit"
```

The human-readable final answer prints to stdout. Progress and diagnostics use
stderr unless you choose JSON output.

## Input

Pass the prompt as an argument:

```bash
interpreter exec "find one bug in src/parser.rs"
```

Read the prompt from stdin:

```bash
cat task.md | interpreter exec -
```

Pipe context into a prompt:

```bash
git diff | interpreter exec "explain this diff and flag risky changes"
```

Attach images to the first prompt:

```bash
interpreter exec -i screenshot.png "describe the UI problem"
```

## Common Flags

| Flag | Purpose |
| ---- | ------- |
| `--json` | Emit newline-delimited JSON events. |
| `--output-schema <file>` | Require the final answer to match a JSON Schema. |
| `--output-last-message, -o <file>` | Write the final assistant message to a file. |
| `--color always|never|auto` | Control ANSI color. |
| `--sandbox <mode>` | Override sandbox mode. |
| `--ask-for-approval <mode>` | Override approval policy. |
| `--profile <name>` | Use a config profile. |
| `--ephemeral` | Do not persist a session record. |
| `--skip-git-repo-check` | Allow running outside a Git repo. |
| `--ignore-user-config` | Skip user config for this run. |
| `--ignore-rules` | Skip execpolicy rules. |
| `--verify` | Run an additional completion-check turn before exiting. |
| `--timeout <seconds>` | Send time remaining reminders during the run. |

## JSON Events

Use `--json` for automation:

```bash
interpreter exec --json "list the files this task would touch"
```

Each line is a JSON event representing progress, tool calls, file changes,
reasoning summaries, or the final message.

## Completion And Exit Status

A zero exit status means the agent turn reached the protocol's completed state.
It does not prove that an arbitrary instruction's intended side effect happened.
For example, a local model can return a normal final message without creating a
file that the prompt requested.

Use `--verify` to ask for an additional completion-check turn when appropriate,
and make automation check its required outputs directly:

```bash
interpreter exec --verify "create NOTES.md summarizing this project"
test -s NOTES.md
```

With local models, also ensure the model server's active context is large enough
for the full agent prompt and tool schemas. See [Local model tool use](/docs/models#local-model-tool-use).

## Structured Output

Pair `--output-schema` with a schema file:

```json schema.json
{
  "type": "object",
  "properties": {
    "risk": { "type": "string" },
    "recommended_fix": { "type": "string" }
  },
  "required": ["risk", "recommended_fix"]
}
```

```bash
interpreter exec --output-schema schema.json \
  "inspect the current diff and return the highest risk"
```

## Resume Exec Work

Continue the most recent non-interactive session:

```bash
interpreter exec resume --last "now apply the plan"
```

Or resume a specific session id:

```bash
interpreter exec resume <SESSION_ID> "continue"
```

Add `--all` to search sessions outside the current working directory.

## Review From Exec

Run a code-review pass without opening the TUI:

```bash
interpreter exec review --uncommitted
interpreter exec review --base main
interpreter exec review --commit abc123
```

For custom review instructions, pass text or `-` to read stdin.

## CI Pattern

Use API-key authentication and keep the sandbox narrow:

```yaml
- name: Review patch
  env:
    OPENAI_API_KEY: ${{ secrets.OPENAI_API_KEY }}
  run: |
    interpreter exec --json --sandbox read-only \
      "review this pull request diff for regressions" \
      < pr.diff > review.jsonl
```
