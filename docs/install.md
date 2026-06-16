---
title: Install
description: Install or update the Open Interpreter CLI on macOS, Linux, or Windows.
---

The public installer downloads the right release for your platform and installs
the managed standalone layout used by Open Interpreter's self-update logic.

<Tabs>
  <Tab title="macOS / Linux">
    ```bash
    curl -fsSL https://www.openinterpreter.com/install | sh
    ```
  </Tab>
  <Tab title="Windows PowerShell">
    ```powershell
    irm https://www.openinterpreter.com/install.ps1 | iex
    ```

    Use the `www.openinterpreter.com` host exactly as shown. Windows
    PowerShell 5.1 does not follow the apex-domain `308 Permanent Redirect`
    from `openinterpreter.com` before evaluating the installer script.
  </Tab>
</Tabs>

Restart your shell after installation, then verify the binary:

```bash
interpreter --version
```

## Requirements

| Item | Notes |
| ---- | ----- |
| macOS | Current release builds target modern macOS versions. |
| Linux | Use a recent glibc-based distribution. |
| Windows | Use PowerShell for installation. WSL is also supported for Linux-style workflows. |
| Git | Recommended for repository-aware sessions, diffs, and reviews. |

## Updating

Standalone installs can check for updates during normal interactive startup.
You can also manage updates explicitly:

```bash
interpreter update status
interpreter update now
interpreter update off
interpreter update on
```

Rerunning the public install command is also supported.

## Build From Source

For local product development, build the release bundle with the repository
script:

```bash
./scripts/build-interpreter-release.sh
```

Do not rely on an ad hoc `cargo build` as a replacement for the release bundle.
The public `interpreter` command is a launcher/router that expects sibling
binaries such as the TUI and app-server binaries to exist next to it.

## Logs

The interactive TUI writes logs under:

```text
~/.openinterpreter/log/
```

For a single run, override the log directory:

```bash
interpreter -c log_dir='"./.interpreter-log"'
```

Use `RUST_LOG` for Rust log filtering when debugging:

```bash
RUST_LOG=info interpreter
```
