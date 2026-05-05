---
title: Non-interactive mode
description: Run Open Interpreter from a script, a pipeline, or CI.
---

Use `interpreter exec` when you want a single prompt to run start-to-finish
without the terminal UI. The final result goes to `stdout`. Progress
streams to `stderr` so you can pipe the result anywhere.

```bash
interpreter exec "summarize the changes in the last commit"
```

## Common flags

| Flag                | What it does                                         |
| ------------------- | ---------------------------------------------------- |
| `--json`            | Emit JSON Lines so other tools can parse the output  |
| `--output-schema`   | Constrain the final answer to a JSON Schema          |
| `--full-auto`       | Allow file edits without prompting                   |
| `--sandbox <mode>`  | Override sandbox mode for this run                   |
| `--ephemeral`       | Skip writing a session record                        |
| `--profile <name>`  | Use a named config profile                           |

Run `interpreter exec --help` for the full list.

## Piping in context

You can feed `stdin` as additional context for the prompt:

```bash
git diff | interpreter exec "explain this diff in plain English"
```

Or pass the whole prompt on `stdin`:

```bash
cat task.md | interpreter exec -
```

## Structured output

Pair `--json` with `--output-schema` to get a structured answer your
script can parse:

```bash
echo '{"type":"object","properties":{"bug":{"type":"string"},"fix":{"type":"string"}},"required":["bug","fix"]}' > schema.json

interpreter exec --json --output-schema schema.json \
  "find one bug in src/parser.rs and propose a fix"
```

The final stdout line is a single JSON object that matches the schema.

## In CI

API keys are the right default for CI. Set them as secrets, then call
`interpreter exec`:

<CodeGroup>
```yaml github-actions.yml
- name: Triage failing tests
  env:
    OPENAI_API_KEY: ${{ secrets.OPENAI_API_KEY }}
  run: |
    interpreter exec --json \
      "look at the failing tests in pytest output and suggest a fix" \
      < pytest.log > suggestion.json
```

```bash shell
OPENAI_API_KEY=$KEY interpreter exec --full-auto \
  "bump the version in pyproject.toml and write a CHANGELOG entry"
```
</CodeGroup>

## Resuming an exec session

Stage longer pipelines by chaining runs:

```bash
interpreter exec "draft a refactor plan for src/parser.rs"
interpreter exec resume --last "now apply the plan"
```

`resume --last` continues the most recent session.

## Logging

`interpreter exec` defaults to `RUST_LOG=error` and prints inline. If you
want more detail:

```bash
RUST_LOG=info interpreter exec "..."
```
