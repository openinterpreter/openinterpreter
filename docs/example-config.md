---
title: Example config
description: A starting point for ~/.openinterpreter/config.toml.
---

Drop this into `~/.openinterpreter/config.toml` and edit to taste. Every
key here is optional. Open Interpreter has working defaults for all of
them.

```toml
# ---------------------------------------------------------------
# Model
# ---------------------------------------------------------------

model = "gpt-5-codex"
model_reasoning_effort = "medium"
personality = "concise"

# Harness behavior, when using harness emulation such as "kimi-cli"
harness_guidance = true

# ---------------------------------------------------------------
# Permissions
# ---------------------------------------------------------------

# Sandbox: "read-only" | "workspace-write" | "danger-full-access"
sandbox_mode = "workspace-write"

# Approvals: "untrusted" | "on-request" | "never"
approval_policy = "on-request"

# Extra paths the workspace sandbox can read
[sandbox]
extra_read_dirs = ["/Users/me/notes"]

# ---------------------------------------------------------------
# Logging
# ---------------------------------------------------------------

log_dir = "~/.openinterpreter/log"

# ---------------------------------------------------------------
# Profiles
# ---------------------------------------------------------------

[profiles.fast]
model = "gpt-5-codex-mini"
model_reasoning_effort = "low"

[profiles.review]
model = "gpt-5-codex"
model_reasoning_effort = "high"
sandbox_mode = "read-only"

# ---------------------------------------------------------------
# MCP servers
# ---------------------------------------------------------------

[mcp_servers.docs]
command = "docs-server"
default_tools_approval_mode = "approve"

[mcp_servers.docs.tools.search]
approval_mode = "prompt"

# ---------------------------------------------------------------
# Features
# ---------------------------------------------------------------

[features]
apps = false
plugins = false
child_agents_md = true

# ---------------------------------------------------------------
# Notify hook
# ---------------------------------------------------------------

[notify]
command = ["osascript", "-e", "display notification \"turn finished\" with title \"Open Interpreter\""]
```

Run with a profile:

```bash
interpreter --profile review
```

Override one value for a single run:

```bash
interpreter -c approval_policy='"never"' "fix the failing tests"
```

See the [Configuration](/docs/config) guide for what each section does.
