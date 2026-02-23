# Safety and Permissions Guide

## macOS Permissions

Desktop GUI automation requires two macOS permissions. Both are per-application (grant to your terminal app: Ghostty, Terminal.app, iTerm2, VS Code, etc.).

### Accessibility

**What it enables**: Mouse movement, clicks, keyboard input (pyautogui)

**How to grant**:
1. System Settings > Privacy & Security > Accessibility
2. Click the lock icon to authenticate
3. Click "+" and add your terminal app
4. If already listed, toggle it off and on again

**Symptoms when missing**: pyautogui operations silently fail or throw "This process is not trusted! Input event monitoring will not be possible until it is added to accessibility clients."

### Screen Recording

**What it enables**: Screen capture (screencapture, pyautogui.screenshot)

**How to grant**:
1. System Settings > Privacy & Security > Screen Recording
2. Click the lock icon to authenticate
3. Click "+" and add your terminal app
4. Restart the terminal app after granting

**Symptoms when missing**: Screenshots are blank (all black or all white), or screencapture produces 0-byte files.

### Verifying Permissions

```bash
python3 ~/.claude/skills/open-interpreter/scripts/oi_permission_check.py
```

This checks:
- pyautogui can read mouse position (Accessibility)
- screencapture produces a non-empty file (Screen Recording)
- tesseract is installed (OCR support)

## Safety Model

### Principles

1. **Human-in-the-loop for destructive actions**: Before clicking Send, Delete, Submit, Confirm, or Purchase buttons, verify with the user. A misclick on a destructive button cannot be undone.

2. **Screenshot-verify pattern**: Take a screenshot before and after every action. This provides an audit trail and catches misclicks early.

3. **No unbounded autonomous loops**: Multi-step GUI workflows should checkpoint with the user. Do not run 50 uninterrupted click sequences without verification.

4. **pyautogui failsafe**: Moving the mouse to any screen corner (0,0 or max,0 or 0,max or max,max) raises `pyautogui.FailSafeException`, which halts execution. This is enabled by default and should not be disabled.

5. **Action logging**: Every script logs its actions to stderr with the `[oi]` prefix. This provides a record of what was done.

### Risk Categories

| Action | Risk | Mitigation |
|--------|------|-----------|
| Screenshot | None (read-only) | — |
| Find text (OCR) | None (read-only) | — |
| Mouse move | Low | Reversible |
| Click (left) | Medium | Screenshot before, verify target |
| Click (right) | Medium | Context menus are dismissible |
| Type text | Medium | Can undo (Cmd+Z) |
| Hotkey | Medium-High | Some hotkeys trigger irreversible actions |
| Click "Delete"/"Send" | High | Require user confirmation |
| Form submission | High | Require user confirmation |

### OS Mode Safety

When using `oi_os_mode.py` (delegated autonomous control), OI runs its own agent loop with `-y` (auto-approve). This means:
- OI will execute actions without asking for confirmation
- The timeout flag provides a hard limit on execution time
- Monitor stderr for OI's action log
- For high-risk tasks, prefer Library mode where Claude Code controls each step

### pyautogui Failsafe

```python
import pyautogui

# Enabled by default — do not disable
pyautogui.FAILSAFE = True  # Default: True

# Pause between actions (seconds)
pyautogui.PAUSE = 0.1  # Default: 0.1
```

To emergency-stop any pyautogui automation, quickly move the mouse to any screen corner. This raises `FailSafeException` and halts the script.

## tesseract Installation

tesseract provides OCR for text detection on screen.

```bash
# macOS
brew install tesseract

# With additional language packs
brew install tesseract-lang

# Verify
tesseract --version
```

The Python binding (`pytesseract`) is installed by `oi_install.sh`. It requires the tesseract CLI to be available in PATH.
