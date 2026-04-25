# Forking Open Interpreter into Your Own CLI

Open Interpreter is intentionally close to the upstream terminal UI at the TUI layer, but it is already
structured so another team can fork it into a branded CLI without rewriting the whole stack.

This is the current minimum map of what to change.

## 1. Replace the visible brand

The main user-facing brand strings are not fully centralized yet. The first concrete places to
change are:

- `codex-rs/tui/src/history_cell.rs`
- `codex-rs/tui/src/status/card.rs`
- `codex-rs/tui/src/slash_command.rs`
- `codex-rs/tui/src/product_branding.rs`

These files currently own:

- the session header symbol and title
- the status card header
- the slash-command descriptions
- the onboarding welcome/auth copy

If you are forking the product, start by replacing the concrete strings in those files.

## 2. Replace the public command and home directory

The single user-facing command and the product-specific home path live in:

- `codex-rs/server-cli/`
- `codex-rs/server-cli/src/home.rs`
- `codex-rs/server-cli/Cargo.toml`

Those are the main places to update when you want:

- a different binary name
- a different default home directory
- different product-specific env overrides

The intended product shape is still:

- one public command users run
- hidden helper binaries managed automatically under that command

Keep that boundary intact unless you are deliberately changing the runtime model.

## 3. Replace the canonical brand accent color

The current canonical markdown accent color lives here:

- `codex-rs/tui/src/markdown_render.rs`

That file now deliberately shares one style helper for:

- markdown links
- ordered-list numerals

If you want to rebrand the TUI color system, start there. The important rule is:

- keep one canonical accent style
- reuse it consistently instead of scattering one-off blues across the UI

That gives forks a clear first place to change the canonical blue without hunting through unrelated
widgets.

## 4. Replace onboarding and model-selection copy

Provider-first onboarding and `/model` now live mostly under:

- `codex-rs/tui/src/onboarding/`
- `codex-rs/tui/src/provider_model_flow.rs`
- `codex-rs/tui/src/chatwidget/model_selection.rs`
- `codex-rs/tui/src/slash_command.rs`

This is where you should update:

- provider picker wording
- model picker wording
- reasoning-effort wording
- slash-command descriptions that still mention the assistant by name

## 5. Replace prompts and long-tail copy

There is still product-specific copy spread through the TUI and launcher.

We are intentionally not pretending this is fully centralized yet.

TODO:
- improve these docs with a more complete prompt-and-copy map
- make branding and prompt replacement easier for model companies shipping their own agent fork
- reduce the number of one-off user-visible strings that require repo-wide search

For now, the practical rule is:

- update the core branding files above first
- then search the repo for your old brand name and clean up the remaining user-facing copy

## 6. Keep the runtime split

If your goal is a branded fork rather than a new runtime architecture, preserve this structure:

- thin public launcher
- one local shared daemon
- upstream TUI substrate

That keeps the maintenance burden lower and makes upstream syncing much easier than inventing a
second renderer or a second execution stack.
