# Open Interpreter 1.0 Prototype

The Open Interpreter 1.0 Prototype is a provider-agnostic terminal coding agent based on
Codex.

## What it does

Open Interpreter is built to work in your terminal:

- run `interpreter` and work in the current directory
- choose the provider and model that fit your workflow
- keep configuration and session state local in `~/.openinterpreter`
- receive standalone app updates automatically by default

## Building Locally

There is exactly one supported way to build a local `interpreter` release from
this checkout:

```shell
./scripts/build-interpreter-release.sh
```

Do not use ad hoc `cargo build` commands for local release work. The
`interpreter` command is a launcher that requires multiple sibling binaries, and
partial builds can leave `interpreter --help` working while plain `interpreter`
fails. The script builds and verifies the complete binary set, then updates the
local `~/.local/bin/interpreter` shim. See [BUILDING.md](./BUILDING.md) for the
full contract.

## Core ideas

- **Provider agnostic:** model and provider choice are first-class parts of the
  product.
- **Memory efficient:** the runtime is designed around a shared local backend so
  many tabs do not have to behave like many fully separate agent runtimes.

## License

Apache-2.0
