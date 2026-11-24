# Branding a Distribution Fork

Open Interpreter inherits internal crate, protocol, and compatibility names from
OpenAI Codex, but those names are not the product identity shown to users. A
distribution fork should change the centralized product identity and packaging
surfaces described here. Do not globally replace `codex` across the repository.

## Product identity

Rust code must get user-facing names and URLs from
`codex-rs/product-info/src/lib.rs`:

- `Product::current()` selects the active distribution.
- `display_name()` supplies the product name used in prose and headings.
- `short_display_name()` supplies the conversational product name.
- `command_name()` supplies the command users should run.
- Product methods supply release, installer, MCP, and memory-documentation URLs.
- Standalone installer commands and their non-interactive environment variable
  also come from this module.

The official Open Interpreter distribution is selected by the installed
package's `codex-package.json` metadata (`variant: "open-interpreter"`). The
`OPEN_INTERPRETER_BRAND` environment variable and the `interpreter`/`i`
executable names are development fallbacks, not the installed-package contract.

For a new branded distribution, add an explicit `Product` variant and its
identity constants rather than changing the meaning of `OpenInterpreter`.
Extend package detection for the new manifest variant, then make every
user-facing call site use the `Product` methods. This preserves upstream Codex
behavior and makes the new brand testable without a global rename.

## Installer and release identity

The Open Interpreter installer exposes these canonical variables:

| Setting              | Variable                               | Official value                    |
| -------------------- | -------------------------------------- | --------------------------------- |
| Display name         | `OPEN_INTERPRETER_PRODUCT_NAME`        | `Open Interpreter`                |
| Primary command      | `OPEN_INTERPRETER_COMMAND_NAME`        | `interpreter`                     |
| Command aliases      | `OPEN_INTERPRETER_ALIAS_COMMAND_NAMES` | `i`                               |
| Release repository   | `OPEN_INTERPRETER_GITHUB_REPO`         | `openinterpreter/openinterpreter` |
| Asset stem           | `OPEN_INTERPRETER_PACKAGE_ASSET_STEM`  | `open-interpreter-package`        |
| Tag prefix           | `OPEN_INTERPRETER_RELEASE_TAG_PREFIX`  | `rust-v`                          |
| Install directory    | `OPEN_INTERPRETER_INSTALL_DIR`         | platform default                  |
| Requested release    | `OPEN_INTERPRETER_RELEASE`             | `latest`                          |
| Non-interactive mode | `OPEN_INTERPRETER_NONINTERACTIVE`      | false unless set                  |
| Product home         | `INTERPRETER_HOME`                     | `~/.openinterpreter`              |

The generic installers still accept older `CODEX_*` names as compatibility
aliases. They are intentionally absent from Open Interpreter help and update
instructions. A new distribution should introduce its own canonical prefix and
retain old aliases only when it needs migration compatibility.

Release packaging is controlled by:

- `scripts/build-interpreter-release.sh`
- `.github/scripts/build-codex-package-archive.sh`
- `.github/workflows/rust-release.yml`
- `.github/workflows/rust-release-publish-existing.yml`
- `scripts/install/install.sh` and `scripts/install/install.ps1`

A fork must keep the manifest variant, entrypoint, archive stem, checksum asset,
release repository, tag prefix, and installer defaults aligned. Package identity
must not depend only on the executable filename, because aliases and symlinks
are expected.

## Names that should remain internal

These are compatibility or implementation surfaces and should not be renamed
as branding work unless the fork deliberately breaks compatibility:

- Rust crate and module names such as `codex-core` and `codex-tui`
- app-server and exec protocol field names
- stable telemetry and IPC identifiers
- package metadata filename `codex-package.json`
- legacy `CODEX_*` environment aliases
- model names such as `gpt-5-codex`
- text describing OpenAI Codex as the upstream project

When an internal name must be shown for diagnostics, label it as a compatibility
name rather than presenting it as the application name.

## Verification checklist

1. Build the packaged distribution, not only a Cargo development binary.
2. Invoke every supported command alias and confirm `--version` and `--help`
   show the distribution name and command.
3. Open the TUI and inspect onboarding, `/status`, `/model`, `/permissions`,
   `/mcp`, `/memories`, update prompts, recovery errors, and session resume
   guidance.
4. Run the installer with `--help` and confirm only the distribution's canonical
   public variables are advertised.
5. Exercise an update and verify its repository, command, non-interactive
   variable, success message, and release-notes URL all match the distribution.
6. Run the identity, TUI snapshot, CLI, and installer tests.
7. Audit remaining literals. Classify every result as upstream attribution,
   compatibility identifier, model name, test fixture, or a bug:

   ```bash
   rg -n 'Codex|codex|CODEX_' codex-rs/tui codex-rs/cli scripts/install
   ```

No unclassified product-facing occurrence should ship.
