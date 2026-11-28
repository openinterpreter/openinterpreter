# Codex package builder

This package contains the implementation behind `scripts/build_codex_package.py`.
The top-level script is the stable executable entry point; these modules keep the
package-building logic split by responsibility.

Run the builder through `just`:

```bash
just assemble-codex-package --help
just assemble-codex-package --variant codex-app-server
just assemble-codex-package --target x86_64-unknown-linux-gnu
```

The builder creates a canonical Codex package directory:

```text
.
├── codex-package.json
├── bin
│   ├── <entrypoint>[.exe]
│   └── codex-code-mode-host[.exe]
├── codex-resources
│   ├── bwrap                             # Linux only
│   ├── zsh/bin/zsh                       # supported Unix targets only
│   ├── llama.cpp/llama-server[.exe]      # open-interpreter variant only
│   ├── codex-command-runner.exe          # Windows only
│   └── codex-windows-sandbox-setup.exe   # Windows only
└── codex-path
    └── rg[.exe]
```

The package directory is the primary artifact. Archive formats such as
`.tar.gz`, `.tar.zst`, and `.zip` are serializations of that directory.

If `--target` is omitted, the builder uses the release target for the current
host platform. On Linux, that default is a musl target to match Codex release
artifacts; pass a GNU Linux target explicitly for native glibc local builds. If
`--package-dir` is omitted, the builder creates a new temporary directory and
prints its path after the package is built.

The `--variant` flag selects the package entrypoint. Supported variants are
`codex`, `codex-app-server`, and `open-interpreter`. The `open-interpreter`
variant packages the upstream multitool as `interpreter` with the `i` alias;
the same entrypoint provides TUI, exec, app-server, and daemon subcommands. The
`--package-version` flag sets the version in
`codex-package.json`; it defaults to `[workspace.package].version` in
`codex-rs/Cargo.toml`.

The `open-interpreter` package requires `--llama-server-bin` pointing to a
prebuilt executable for the selected target. The builder fails before running
Cargo when it is absent; it does not install a server on the user's machine or
copy a release archive as if it were an executable. Supply a verified,
platform-matched upstream binary through the release job and retain its version
and digest in that job's provenance. Package metadata records the staged
`codex-resources/llama.cpp/llama-server[.exe]` path.
For a local build, pass the platform-matched binary to
`scripts/build-interpreter-release.sh --llama-server-bin /path/to/llama-server
--llama-server-sha256 EXPECTED_64_HEX_DIGEST`.
The archive assembler also accepts `--llama-server-bin`; release automation
must supply a pinned, verified binary for each target before selecting the
`open-interpreter` bundle. No executable is fetched implicitly at install time.

## Source-built artifacts

Artifacts built from this repository are built by the package builder in one
grouped `cargo build` command per package when they are needed and no prebuilt
override was provided:

- all targets: the selected entrypoint, unless `--entrypoint-bin` is provided
- all targets: `codex-code-mode-host`, unless `--code-mode-host-bin` is provided
- Linux targets: `bwrap`, unless `--bwrap-bin` is provided
- Windows targets: `codex-command-runner` and `codex-windows-sandbox-setup`,
  unless the corresponding prebuilt helper flags are provided

The default cargo profile is `dev-small` because local iteration should favor
fast, small builds. Release jobs should pass `--cargo-profile release` and an
explicit target. Release jobs that already built and signed/notarized the
entrypoint should pass `--entrypoint-bin` so the package contains that exact
binary instead of rebuilding it.

Release jobs should likewise pass `--code-mode-host-bin` so the package contains
the signed host executable beside the signed entrypoint.

Release jobs that already built package resource binaries should also pass the
corresponding resource flags: `--bwrap-bin` for Linux packages, and
`--codex-command-runner-bin` plus `--codex-windows-sandbox-setup-bin` for
Windows packages. This keeps package archive creation as a pure staging step
after signing instead of rebuilding resources.

For package variants that enable the V8 runtime on Darwin or Linux, the builder
downloads and verifies the matching Codex-built V8 release pair before invoking
Cargo and sets `RUSTY_V8_ARCHIVE` plus `RUSTY_V8_SRC_BINDING_PATH` for that
build. Variants that do not enable the V8 runtime skip this step. Windows targets
keep Cargo's release-build MSVC artifact path. Explicit overrides remain
authoritative when both variables are already set. Set `V8_FROM_SOURCE=1` to
leave the build with the `v8` crate source-build path.

`rg` is not built from this repository, so the builder fetches it from the
DotSlash manifest at `scripts/codex_package/rg`. Downloaded archives are cached
under `$TMPDIR/codex-package/<target>-rg` and are reused only after the recorded
size and SHA-256 digest have been verified. Pass `--rg-bin` to use a local
ripgrep executable instead.

The patched zsh fork used by `shell_zsh_fork` is fetched from the DotSlash
manifest at `scripts/codex_package/codex-zsh` when the selected target has a
matching prebuilt artifact. Downloaded archives are cached under
`$TMPDIR/codex-package/<target>-zsh` and installed at
`codex-resources/zsh/bin/zsh`. Pass `--zsh-bin` to package a prebuilt, signed
executable, or `--zsh-manifest` to use a different DotSlash manifest, such as
the manifest published with a standalone zsh artifact release.
