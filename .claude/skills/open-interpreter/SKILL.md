---
name: open-interpreter
description: Desktop GUI automation via OpenInterpreter — mouse, keyboard, screenshot,
  and OCR control for native macOS/Linux applications. Three modes: Library (Claude
  reasons, OI executes), OS subprocess (full autonomous computer use), and Local agent
  (Ollama, offline). This skill should be used when interacting with desktop apps that
  have no CLI or API, automating GUI workflows, reading screen content via OCR, or
  controlling mouse/keyboard.
---

# OpenInterpreter — Desktop GUI Automation

Desktop control for Claude Code via [OpenInterpreter](https://github.com/OpenInterpreter/open-interpreter) (62k stars, AGPL-3.0). Mouse, keyboard, screenshot, and OCR primitives backed by pyautogui + pytesseract.

## Mode Selection

| Mode | LLM | Script | Best For |
|------|-----|--------|----------|
| **Library** | Claude Code (native) | Individual scripts below | Surgical GUI actions — Claude sees screenshots, reasons, dispatches actions |
| **OS subprocess** | Claude API (via OI) | `oi_os_mode.py` | Full autonomous computer use — delegate entire GUI tasks |
| **Local agent** | Ollama (offline) | `oi_os_mode.py --local` | Offline computer use, no API costs, privacy-sensitive tasks |

Use Library mode by default. Use OS subprocess to delegate self-contained GUI tasks. Use Local agent when offline or to avoid API costs.

## Installation

Run once:

```bash
.claude/skills/open-interpreter/scripts/oi_install.sh
```

Installs `open-interpreter[os]` via uv, verifies pyautogui and tesseract, checks macOS permissions.

**macOS permissions** (one-time, manual):
- System Settings > Privacy & Security > **Accessibility** > add terminal app (Ghostty/Terminal/iTerm2)
- System Settings > Privacy & Security > **Screen Recording** > add terminal app

Verify permissions:

```bash
python3 .claude/skills/open-interpreter/scripts/oi_permission_check.py
```

## Library Mode: The Screenshot Loop

The core pattern for GUI automation:

```
1. Take screenshot   →  oi_screenshot.py
2. Read PNG          →  Claude Read tool (native vision)
3. Decide action     →  Claude reasoning
4. Execute action    →  oi_click.py / oi_type.py
5. Verify            →  Take another screenshot
6. Loop until done
```

### Scripts

**`oi_screenshot.py`** — Capture screen, return file path with Retina metadata

```bash
python3 .claude/skills/open-interpreter/scripts/oi_screenshot.py
python3 .claude/skills/open-interpreter/scripts/oi_screenshot.py --region 0,0,800,600
python3 .claude/skills/open-interpreter/scripts/oi_screenshot.py --active-window
```

Output (3 lines):
```
/tmp/oi_screenshot_1708789200.png
SCALE_FACTOR=2
SCREEN_SIZE=1512x982
```

**`oi_click.py`** — Mouse click by coordinates or OCR text

```bash
python3 .claude/skills/open-interpreter/scripts/oi_click.py --x 450 --y 300
python3 .claude/skills/open-interpreter/scripts/oi_click.py --x 900 --y 600 --image-coords
python3 .claude/skills/open-interpreter/scripts/oi_click.py --text "Submit"
python3 .claude/skills/open-interpreter/scripts/oi_click.py --x 450 --y 300 --double
python3 .claude/skills/open-interpreter/scripts/oi_click.py --x 450 --y 300 --right
```

- `--image-coords`: auto-divides by Retina scale factor (use when coordinates come from screenshot image pixels)
- `--text`: OCR-based — screenshots, finds text via pytesseract, clicks center of match

**`oi_type.py`** — Keyboard input

```bash
python3 .claude/skills/open-interpreter/scripts/oi_type.py --text "hello world"
python3 .claude/skills/open-interpreter/scripts/oi_type.py --key enter
python3 .claude/skills/open-interpreter/scripts/oi_type.py --hotkey command space
python3 .claude/skills/open-interpreter/scripts/oi_type.py --text "search" --method typewrite
```

- Default text input: clipboard-paste (Cmd+V) for speed and Unicode safety
- `--method typewrite`: character-by-character (use when clipboard is needed for other purposes)
- `--hotkey`: AppleScript on macOS for reliable modifier key handling

**`oi_find_text.py`** — OCR screen reading

```bash
python3 .claude/skills/open-interpreter/scripts/oi_find_text.py --text "Submit"
python3 .claude/skills/open-interpreter/scripts/oi_find_text.py --text "Price" --screenshot /tmp/ss.png
```

Returns JSON array: `[{"text": "Submit", "x": 450, "y": 300, "w": 80, "h": 24, "confidence": 95}]`

**`oi_computer.py`** — Unified dispatch for all actions

```bash
python3 .claude/skills/open-interpreter/scripts/oi_computer.py screenshot
python3 .claude/skills/open-interpreter/scripts/oi_computer.py click --x 450 --y 300
python3 .claude/skills/open-interpreter/scripts/oi_computer.py type --text "hello"
python3 .claude/skills/open-interpreter/scripts/oi_computer.py find --text "Submit"
python3 .claude/skills/open-interpreter/scripts/oi_computer.py scroll --clicks 3
python3 .claude/skills/open-interpreter/scripts/oi_computer.py mouse-position
python3 .claude/skills/open-interpreter/scripts/oi_computer.py screen-size
```

### Retina Display Handling

macOS Retina displays render at 2x (or 3x) scaling. Screenshot image pixels differ from screen coordinates:

| Metric | Example (14" MBP) |
|--------|-------------------|
| Image pixels (screenshot) | 3024 x 1964 |
| Screen coordinates (pyautogui) | 1512 x 982 |
| Scale factor | 2x |

When estimating click targets from a screenshot image, use `--image-coords` on `oi_click.py` to auto-divide by the scale factor. The `oi_screenshot.py` output includes `SCALE_FACTOR` metadata.

## OS Mode: Delegate Full Tasks

For self-contained GUI tasks, delegate to OI's full agent loop:

```bash
python3 .claude/skills/open-interpreter/scripts/oi_os_mode.py "Open Calculator and compute 2+2"
python3 .claude/skills/open-interpreter/scripts/oi_os_mode.py --provider anthropic "Change the desktop wallpaper"
```

OI runs its own screenshot → analyze → act loop using the Claude API. Requires `ANTHROPIC_API_KEY`.

## Local Mode: Offline Computer Use

Run OI with a local vision model via Ollama:

```bash
python3 .claude/skills/open-interpreter/scripts/oi_os_mode.py --local "What apps are open?"
```

Prerequisites:
1. Ollama running: `ollama serve`
2. Vision model pulled: `ollama pull llama3.2-vision`

Limitation: Local models use OI's classic code-execution mode, not the screenshot-driven OS Mode (which requires Claude 3.5 Sonnet). Local mode generates and executes code to accomplish GUI tasks rather than using pixel-level screenshot analysis.

## Common Recipes

### Open an App via Spotlight

```bash
python3 scripts/oi_type.py --hotkey command space
sleep 0.5
python3 scripts/oi_type.py --text "Calculator"
sleep 0.3
python3 scripts/oi_type.py --key enter
```

### Read Text from Screen

```bash
python3 scripts/oi_screenshot.py > /tmp/ss_meta.txt
python3 scripts/oi_find_text.py --text "Total" --screenshot "$(head -1 /tmp/ss_meta.txt)"
```

### Click a Button by Label

```bash
python3 scripts/oi_click.py --text "Save"
```

### Fill a Form Field

```bash
python3 scripts/oi_click.py --text "Email"
python3 scripts/oi_type.py --text "user@example.com"
python3 scripts/oi_type.py --key tab
python3 scripts/oi_type.py --text "password123"
```

## Safety

1. **Confirm before destructive actions** — before clicking Send, Delete, Submit, or Confirm buttons, verify with the user
2. **Screenshot before and after** every action for verification
3. **No unbounded autonomous loops** — confirm with user between multi-step GUI workflows
4. **pyautogui failsafe** — moving mouse to any screen corner raises `pyautogui.FailSafeException` (enabled by default)
5. **Action logging** — every script logs actions to stderr: `[oi] click at (450, 300) button=left`

## Troubleshooting

| Symptom | Fix |
|---------|-----|
| `oi_screenshot.py` returns black image | Grant Screen Recording permission to terminal app |
| `oi_click.py` / `oi_type.py` no effect | Grant Accessibility permission to terminal app |
| OCR finds no text | Verify tesseract: `which tesseract && tesseract --version` |
| Retina coordinates off by 2x | Use `--image-coords` flag on `oi_click.py` |
| `oi_find_text.py` low confidence | Try larger text, ensure screen is not obstructed |
| OS Mode hangs | Verify `ANTHROPIC_API_KEY` is set, check OI stderr output |
| Local mode fails | Verify Ollama running (`ollama list`) and model pulled |

## Reference Documentation

| File | Contents |
|------|----------|
| `references/computer-api.md` | OI Computer API reference — mouse, keyboard, display, clipboard |
| `references/os-mode.md` | OS Mode usage, provider configuration, agent loop architecture |
| `references/safety-and-permissions.md` | macOS permissions guide, safety model, failsafe configuration |
