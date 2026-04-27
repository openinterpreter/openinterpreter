# Open Interpreter 1.0

Open Interpreter 1.0 is a provider-agnostic terminal coding agent based on
Codex.

It keeps the terminal workflow strong, makes multi-tab use more memory
efficient, and supports native harness emulation for systems like Claude Code.

## What it does

Open Interpreter is built for practical software work in the terminal:

- run `interpreter` and work in the current directory
- choose the provider and model that fit your workflow
- keep configuration and session state local in `~/.openinterpreter`
- receive standalone app updates automatically by default
- use long-running sessions without treating each tab as a separate product

## Core ideas

- **Based on Codex:** Open Interpreter builds on the mature Codex terminal
  experience instead of replacing it with a separate UI model.
- **Provider agnostic:** model and provider choice are first-class parts of the
  product.
- **Memory efficient:** the runtime is designed around a shared local backend so
  many tabs do not have to behave like many fully separate agent runtimes.
- **Harness emulation:** Open Interpreter can emulate harness behavior such as
  Claude Code natively in Rust instead of depending on external CLIs in the
  product runtime.
- **Automatic updates:** standalone installs stage new releases in the
  background and use them on the next launch.

## Product goals

- provider agnostic model access
- reliable long-running terminal sessions
- efficient multi-tab usage
- clean onboarding and model selection
- clear tool, approval, and session behavior
- safe default-on updates with an opt-out

## User model

- run `interpreter`
- select a provider, model, and reasoning level in the UI
- work directly in the current project
- keep state under `~/.openinterpreter`

## Scope

This README describes the product itself.

Open Interpreter should read here as:

- a terminal coding agent
- based on Codex
- provider agnostic
- memory efficient
- capable of native harness emulation
