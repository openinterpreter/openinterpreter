---
title: Permissions
description: Define reusable least-privilege profiles for filesystem and network access.
---

Permission profiles are the newer, finer-grained alternative to the older
`sandbox_mode` and `sandbox_workspace_write` settings. Use one system at a
time: if active config sets `sandbox_mode` or you pass `--sandbox`, the older
sandbox settings take precedence.

## Built-In Profiles

| Profile               | Purpose                                   |
| --------------------- | ----------------------------------------- |
| `:read-only`          | Read-only local command execution.        |
| `:workspace`          | Read/write access inside workspace roots. |
| `:danger-full-access` | No local sandbox restriction.             |

Select a profile:

```toml
default_permissions = ":workspace"
```

## Custom Profile

```toml
default_permissions = "project-edit"

[permissions.project-edit.workspace_roots]
"~/code/app" = true
"~/code/shared-lib" = true

[permissions.project-edit.filesystem]
":minimal" = "read"

[permissions.project-edit.filesystem.":workspace_roots"]
"." = "write"
".devcontainer" = "read"
"**/*.env" = "deny"

[permissions.project-edit.network]
enabled = true

[permissions.project-edit.network.domains]
"api.openai.com" = "allow"
"**.github.com" = "allow"
"tracking.example.com" = "deny"
```

This profile gives common runtime paths read access, allows writes in workspace
roots, keeps `.devcontainer/` read-only, denies `.env` files, and limits
network traffic to selected domains.

## Filesystem Rules

| Access  | Meaning                                                                      |
| ------- | ---------------------------------------------------------------------------- |
| `read`  | Commands can read and list files.                                            |
| `write` | Commands can read, create, update, rename, and delete files.                 |
| `deny`  | Commands cannot read or write matching paths. Deny wins over broader grants. |

Supported roots include:

| Root               | Meaning                                                 |
| ------------------ | ------------------------------------------------------- |
| `:minimal`         | Runtime paths needed by common tools.                   |
| `:workspace_roots` | The current workspace roots plus profile-defined roots. |
| `:tmpdir`          | The active temporary directory.                         |
| `:root`            | The filesystem root. Use sparingly.                     |
| `/absolute/path`   | A concrete absolute path.                               |
| `~/path`           | A path under the user's home.                           |

Use exact paths where possible. Deny globs such as `**/*.env` are useful for
secret-bearing files; on some platforms a `glob_scan_max_depth` value may be
needed to bound startup scanning.

## Network Rules

Network access starts disabled in permission profiles. Enable it explicitly:

```toml
[permissions.project-edit.network]
enabled = true
```

Then allow or deny domains:

```toml
[permissions.project-edit.network.domains]
"example.com" = "allow"
"*.example.com" = "allow"
"**.example.com" = "allow"
"ads.example.com" = "deny"
```

Patterns:

| Pattern          | Meaning                                |
| ---------------- | -------------------------------------- |
| `example.com`    | Exact host.                            |
| `*.example.com`  | Subdomains only.                       |
| `**.example.com` | Apex and subdomains.                   |
| `*`              | Broad public allow. Use intentionally. |

Local and private network destinations are guarded separately. Allow literal
targets like `localhost` or `127.0.0.1` when you need them.

## Unix Sockets

Unix socket allow rules are local escape hatches for tools such as Docker:

```toml
[permissions.project-edit.network.unix_sockets]
"/var/run/docker.sock" = "allow"
```

Use them only when the workflow genuinely needs that local service.

## Scope

Permission profiles govern local sandboxed command execution. App connectors,
MCP servers, browser/computer-use surfaces, approved escalations, and remote
services have their own controls.
