---
title: Install
description: Install the Open Interpreter CLI on macOS, Linux, or Windows.
---

The fastest way to get Open Interpreter is the install script. It downloads
the right binary for your platform and puts `interpreter` on your `PATH`.

<Tabs>
  <Tab title="macOS / Linux">
    ```bash
    curl -fsSL https://openinterpreter.com/install.sh | sh
    ```
  </Tab>
  <Tab title="Windows (PowerShell)">
    ```powershell
    irm https://openinterpreter.com/install.ps1 | iex
    ```
  </Tab>
</Tabs>

After it finishes, restart your shell and run:

```bash
interpreter --version
```

## System requirements

| Item              | Minimum                                                         |
| ----------------- | --------------------------------------------------------------- |
| Operating system  | macOS 12+, Ubuntu 20.04+/Debian 10+, or Windows 11 (via WSL2)   |
| RAM               | 4 GB (8 GB recommended)                                         |
| Git               | 2.23+ if you want the built-in PR helpers                       |

## Updating

Open Interpreter checks for updates in the background and stages new
releases automatically. To update right now:

```bash
/update
```

Run it from inside a session, or rerun the install script.

## DotSlash

Releases include a [DotSlash](https://dotslash-cli.com/) descriptor named
`interpreter`. Commit it to a repo and every contributor runs the same
version of the binary, no matter their platform.

## Build from source

You only need this if you want to develop on the CLI itself.

<Steps>
  <Step title="Clone the repo">
    ```bash
    git clone https://github.com/openinterpreter/open-interpreter.git
    cd open-interpreter/codex-rs
    ```
  </Step>
  <Step title="Install the Rust toolchain">
    ```bash
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source "$HOME/.cargo/env"
    rustup component add rustfmt clippy
    ```
  </Step>
  <Step title="Install workspace helpers">
    ```bash
    cargo install just
    cargo install --locked cargo-nextest
    ```
  </Step>
  <Step title="Build and run">
    ```bash
    cargo build
    cargo run --bin interpreter -- "explain this codebase to me"
    ```
  </Step>
</Steps>

The root `justfile` has the everyday workflow shortcuts:

```bash
just fmt
just fix -p <crate-you-touched>
just test
```

Avoid `--all-features` for routine local runs. It bloats `target/` and slows
the build down for little benefit. Use it only when you specifically need
full feature coverage.

## Verbose logging

Open Interpreter is written in Rust and honors the `RUST_LOG` environment
variable.

The TUI defaults to `RUST_LOG=codex_core=info,codex_tui=info,codex_rmcp_client=info`
and writes logs to:

```
~/.openinterpreter/log/interpreter-tui.log
```

Tail it in another terminal while you work:

```bash
tail -F ~/.openinterpreter/log/interpreter-tui.log
```

For a single run, override the directory with `-c log_dir=...`:

```bash
interpreter -c log_dir=./.interpreter-log
```

`interpreter exec` defaults to `RUST_LOG=error` and prints messages inline,
so you do not need a separate log file.

See the [`RUST_LOG` reference](https://docs.rs/env_logger/latest/env_logger/#enabling-logging)
for the full filter syntax.
