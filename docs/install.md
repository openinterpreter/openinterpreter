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
  </Tab>
</Tabs>

Restart your shell after installation, then verify the binary:

```bash
interpreter --version
```

## Requirements

| Item    | Notes                                                                                |
| ------- | ------------------------------------------------------------------------------------ |
| macOS   | Current release builds target modern macOS versions.                                 |
| Linux   | Use a recent 64-bit distribution. Release archives use musl for broad compatibility. |
| Windows | Use PowerShell for installation. WSL is also supported for Linux-style workflows.    |
| Git     | Recommended for repository-aware sessions, diffs, and reviews.                       |

## Updating

Standalone installs can check for updates during normal interactive startup.
You can also run the installer for the latest release explicitly:

```bash
interpreter update
```

Set `check_for_update_on_startup = false` in your configuration to disable
automatic startup checks.

Rerunning the public install command is also supported.

## Uninstalling

These commands remove the managed standalone installation created by the
public installer. They keep the user data under `.openinterpreter`, including
your configuration, sessions, logs, and file-stored credentials, so you can
reinstall without losing them.

<Tabs>
  <Tab title="macOS">
    ```bash
    for name in interpreter i codex-code-mode-host; do
      path="$HOME/.local/bin/$name"
      case "$(readlink "$path" 2>/dev/null || true)" in
        "$HOME/.openinterpreter/packages/standalone/"*) rm -f "$path" ;;
      esac
    done
    rm -rf "$HOME/.openinterpreter/packages/standalone"
    ```

    If the installer added a marked `Open Interpreter installer` block to
    `~/.zprofile` or `~/.bash_profile`, you can remove that block. It is also
    safe to leave `~/.local/bin` on `PATH`, especially if other tools use it.

  </Tab>
  <Tab title="Linux">
    ```bash
    for name in interpreter i codex-code-mode-host; do
      path="$HOME/.local/bin/$name"
      case "$(readlink "$path" 2>/dev/null || true)" in
        "$HOME/.openinterpreter/packages/standalone/"*) rm -f "$path" ;;
      esac
    done
    rm -rf "$HOME/.openinterpreter/packages/standalone"
    ```

    If the installer added a marked `Open Interpreter installer` block to
    `~/.bashrc`, `~/.zshrc`, or `~/.profile`, you can remove that block. It is
    also safe to leave `~/.local/bin` on `PATH`, especially if other tools use
    it.

  </Tab>
  <Tab title="Windows PowerShell">
    ```powershell
    $binDir = Join-Path $env:LOCALAPPDATA "Programs\Open Interpreter\bin"
    $interpreterHome = Join-Path $env:USERPROFILE ".openinterpreter"
    $standaloneRoot = Join-Path $interpreterHome "packages\standalone"

    if (Test-Path -LiteralPath $binDir) {
        $binItem = Get-Item -LiteralPath $binDir -Force
        $binTarget = [string]$binItem.Target
        $isManagedJunction =
            ($binItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -and
            $binTarget.StartsWith($standaloneRoot, [StringComparison]::OrdinalIgnoreCase)

        if (-not $isManagedJunction) {
            throw "Refusing to remove $binDir because it is not an Open Interpreter managed junction."
        }

        Remove-Item -LiteralPath $binDir -Recurse -Force
    }

    Remove-Item -LiteralPath $standaloneRoot -Recurse -Force -ErrorAction SilentlyContinue

    $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
    if (-not [string]::IsNullOrWhiteSpace($userPath)) {
        $nextPath = ($userPath -split ";" | Where-Object {
            -not [string]::Equals($_.TrimEnd("\"), $binDir.TrimEnd("\"), [StringComparison]::OrdinalIgnoreCase)
        }) -join ";"
        [Environment]::SetEnvironmentVariable("Path", $nextPath, "User")
    }
    ```

    Open a new PowerShell window after uninstalling so it picks up the updated
    user `PATH`.

  </Tab>
</Tabs>

If you set `OPEN_INTERPRETER_INSTALL_DIR`, `INTERPRETER_HOME`,
`CODEX_INSTALL_DIR`, or `CODEX_HOME` when installing, substitute those custom
locations for the defaults above.

<Warning>
The commands above intentionally preserve your user data. To also erase local
configuration, sessions, logs, and file-stored credentials, delete
`~/.openinterpreter` on macOS or Linux, or
`$env:USERPROFILE\.openinterpreter` on Windows, only after you have backed up
anything you need. This cannot be undone. It does not remove credentials kept
in an OS keyring or environment variables.
</Warning>

## Build From Source

For local product development, build the release bundle with the repository
script:

```bash
./scripts/build-interpreter-release.sh
```

Do not rely on an ad hoc `cargo build` as a replacement for the release bundle.
The script builds and stages the same managed package layout used by the public
installer, including the entrypoint, bundled resources, and platform helpers
that install-context detection and self-updates rely on.

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
