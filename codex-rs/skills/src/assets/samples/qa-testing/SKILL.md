---
name: qa-testing
description: Verify your work by actually operating the app or website you changed, instead of assuming it works. Strongly recommended whenever you build, modify, or debug a web app, website, or desktop GUI app. Drive real browsers with the agent-browser CLI (navigate, click, fill, snapshot, screenshot) and native macOS apps with the cua-driver CLI (snapshot the accessibility tree, click/type by element). These aren't bundled - you install them on demand (one network download, no Node required), gated by the host's normal command-approval.
---

# QA testing - verify by actually driving it

After you build or change an app or website, **don't assume it works - drive it
and check.**

## 1. Network is required - check it first

Installing these tools and (for web) loading pages need outbound network, which
sandboxes block by default. Check before anything else:

```bash
curl -fsI https://github.com >/dev/null 2>&1 && echo "network ok" || echo "network blocked"
```

If it prints **network blocked**, stop and tell the user - don't attempt offline
workarounds:

> I need network access to install and run the testing tools. Run **/permissions**
> and choose an access level that allows network (Full Access), then ask me again.

## 2. Web apps / websites -> agent-browser (native Rust binary - no Node)

Install the prebuilt binary directly. Works on any macOS/Linux with `curl` - no
Node, no Homebrew, no version manager (avoid `npm i -g`; it breaks under
Volta/nvm/fnm):

```bash
if ! command -v agent-browser >/dev/null; then
  os=$(uname -s | tr '[:upper:]' '[:lower:]'); m=$(uname -m)
  case "$os/$m" in
    darwin/arm64)              asset=agent-browser-darwin-arm64 ;;
    darwin/x86_64)             asset=agent-browser-darwin-x64 ;;
    linux/aarch64|linux/arm64) asset=agent-browser-linux-arm64 ;;
    linux/x86_64)              asset=agent-browser-linux-x64 ;;
  esac
  mkdir -p ~/.local/bin
  curl -fL "https://github.com/vercel-labs/agent-browser/releases/latest/download/$asset" -o ~/.local/bin/agent-browser
  chmod +x ~/.local/bin/agent-browser
fi
agent-browser install            # one-time: downloads a Chrome build
agent-browser skills get core    # the real usage guide (maintained by the tool)
```

(Convenience alternatives: `brew install agent-browser`, or `cargo install
agent-browser`.) Then: `agent-browser open <url>` -> `snapshot -i` -> act on the
`@eN` refs -> re-snapshot.

## 3. Native macOS apps -> cua-driver (Cua AI)

```bash
command -v cua-driver >/dev/null || /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/trycua/cua/main/libs/cua-driver/scripts/install.sh)"
cua-driver list-tools            # the real tool reference (maintained by the tool)
```

The first GUI action triggers a one-time macOS Accessibility / Screen-Recording
prompt - the user grants it in System Settings once. Run `cua-driver serve` so
element state persists across calls. Defer to cua-driver's own installed skill
for the full workflow.

## Principles

- **Network first.** Check it; if blocked, tell the user to open `/permissions`.
- **`command -v` before installing**; don't reinstall if present.
- **No Node dependency** - use the direct binary download (or brew/cargo), never
  rely on `npm i -g`.
- **Defer to each tool's own docs** (`agent-browser skills get core`,
  `cua-driver list-tools`) - they're the source of truth, current with the
  installed version.
- **Snapshot -> act -> re-snapshot** to confirm each step landed; if nothing
  changed, it failed - say so, don't claim success.
- **Confirm before consequential actions** (purchases, messages, form
  submissions, deletions) - get explicit user intent for that step.
