# Open Interpreter

Open Interpreter is a provider-agnostic coding agent for your terminal.

It is based on the OpenAI Codex CLI surface, with Open Interpreter defaults for
provider choice, local state, and the interactive TUI.

---

## Installation

```bash
# macOS and Linux
curl -fsSL https://openinterpreter.com/install | sh
```

```powershell
# Windows
irm https://openinterpreter.com/install.ps1 | iex
```

After installation, start Open Interpreter in any project:

```bash
interpreter
```

`i` is a built-in shorthand for `interpreter` — `i` and `interpreter` are
interchangeable, so you can just type:

```bash
i
```

You will be prompted to sign in or configure a provider, then Open Interpreter
will work in the current directory.

## What You Can Do

- Ask questions about the codebase in the current directory.
- Make edits, run commands, and inspect files from the terminal.
- Switch providers and models without changing tools.
- Use `interpreter exec` for non-interactive scripting.
- Keep config and session state local under `~/.openinterpreter`.

## CLI Compatibility

`interpreter` preserves the Codex CLI command surface under the Open
Interpreter name. Subcommands, flags, and non-interactive flows should behave
like their Codex CLI equivalents.

The intentional difference is bare startup: running `interpreter` without a
subcommand starts Open Interpreter's app-server-backed interactive TUI.

For programmatic integrations, use OpenAI's Codex SDK and point it at
`interpreter` or `interpreter-app-server`. See [the SDK docs](./docs/sdk.md).

## Building Locally

For local release builds from this checkout, use the repository build script:

```bash
./scripts/build-interpreter-release.sh
```

Do not build only `interpreter` by itself for local release testing. The
user-facing binary is a launcher/router and depends on sibling release
binaries at runtime: `interpreter-tui`, `interpreter-root-tui`,
`interpreter-app-server`, and `interpreter-exec`.

See [BUILDING.md](./BUILDING.md) for the full local build contract.

## License

Apache-2.0
