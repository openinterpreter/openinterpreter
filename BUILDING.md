# Building Open Interpreter

There is exactly one supported way to build a local `interpreter` release from
this checkout:

```shell
./scripts/build-interpreter-release.sh
```

Run it from the `oi1/` repository root. Do not replace it with an ad hoc Cargo,
Bazel, or just command for local release work.

The script changes into `codex-rs/` before invoking Cargo so the repo-local
`rust-toolchain.toml`, Cargo target directory, and release profile are the ones
used. It builds the complete release binary set required by `interpreter`:

- `codex-rs/target/release/interpreter`
- `codex-rs/target/release/interpreter-tui`
- `codex-rs/target/release/interpreter-app-server`
- `codex-rs/target/release/interpreter-root-tui`

It then installs or updates `~/.local/bin/interpreter` so typing `interpreter`
runs the release launcher from this checkout.

Do not build only `interpreter` by itself for local use. The launcher delegates
to sibling binaries at runtime, so a partial build can pass `interpreter --help`
while plain `interpreter` still fails.

After the script finishes, verify the active command:

```shell
command -v interpreter
interpreter --version
```

If `command -v interpreter` does not print `~/.local/bin/interpreter`, put
`~/.local/bin` earlier in your `PATH`.
