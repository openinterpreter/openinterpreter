# Codex MCP runtime

This crate manages Codex's MCP connections, tool calls, and their captured authority.

## Sandbox request metadata

An MCP server advertises the experimental `codex/sandbox-state` capability to
receive sandbox metadata on tool calls. The deprecated capability name
`codex/sandbox-state-meta` opts into the same contract during migration.

Codex supplies these entries in the request's `_meta` object:

| Key                        | Value                                                                                                                    |
| -------------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| `codex/sandbox-state`      | Opaque policy to pass to `codex sandbox --sandbox-state-json`.                                                           |
| `codex/sandbox-state-meta` | An identical copy of that policy under the deprecated name.                                                              |
| `codex/sandbox-launcher`   | When available, an object with an absolute `codexExecutable` path to a verified full Codex CLI on the MCP server's host. |

Use the launcher path to invoke `sandbox --sandbox-state-json` with the entire
state value serialized as one argument, followed by `--` and the command to run.
The state contains neither launcher discovery data nor deprecation messages.
Preserve unknown policy fields by forwarding the original value rather than
deserializing and reconstructing it. The two state names are aliases, not separate
policies; new consumers should prefer `codex/sandbox-state`.

Launcher metadata is currently available only for local stdio servers. An HTTP
endpoint, including loopback, or a remote executor does not establish a shared
filesystem with the harness. Codex omits the launcher when its path is unknown,
not representable as UTF-8, or fails the cached capability probe. Absence does not
authorize a PATH search or unsandboxed fallback. Servers that require a launcher
must fail closed or require an explicit operator opt-out.

The state and launcher describe the issuing call. Retain both when starting a
long-lived subprocess or caching policy-bound results; reject changed authority
before reusing them. Tool arguments supplied by the model are not authority.
See [`sandbox_metadata.rs`](src/sandbox_metadata.rs) for the wire types and
[`binding.rs`](src/binding.rs) for execution-environment selection.
