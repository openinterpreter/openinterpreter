# open-interpreter — Claude Code Skill

A [Claude Code skill](https://code.claude.com/docs/en/skills) for desktop GUI automation, built on top of Open Interpreter's Computer API. Provides mouse, keyboard, screenshot, and OCR control for native macOS/Linux applications that have no CLI or API.

## What is this?

[Claude Code](https://github.com/anthropics/claude-code) is Anthropic's terminal-based AI coding tool. It reads `.claude/skills/` directories for specialized capabilities. This skill gives Claude Code the ability to interact with desktop GUIs by wrapping Open Interpreter's pyautogui + pytesseract primitives in standalone scripts.

## When to Use

- Interacting with desktop apps (System Preferences, Calculator, browsers, any GUI)
- Automating GUI workflows (form filling, menu navigation, data extraction)
- Reading screen content via OCR (finding buttons, labels, prices, status text)
- Controlling mouse and keyboard programmatically

## Modes

| Mode | LLM | Script | Best For |
|------|-----|--------|----------|
| **Library** | Claude Code (native) | Individual scripts | Surgical GUI actions — Claude sees screenshots, reasons, dispatches |
| **OS subprocess** | Claude API (via OI) | `oi_os_mode.py` | Delegating entire GUI tasks to OI's agent loop |
| **Local agent** | Ollama (offline) | `oi_os_mode.py --local` | Offline computer use, no API costs |

Use Library mode by default. OS subprocess for self-contained GUI tasks. Local agent when offline.

## Prerequisites

- Python 3.10+
- [uv](https://github.com/astral-sh/uv) package manager
- macOS: Accessibility + Screen Recording permissions for terminal app
- tesseract (`brew install tesseract`)

## Installation

To use this skill, copy the folder into your Claude Code skills directory:

```bash
cp -r .claude/skills/open-interpreter ~/.claude/skills/open-interpreter
```

Then run the install script:

```bash
~/.claude/skills/open-interpreter/scripts/oi_install.sh
```

Verify permissions:

```bash
python3 ~/.claude/skills/open-interpreter/scripts/oi_permission_check.py
```

## Directory Structure

```
open-interpreter/
├── SKILL.md                          # Skill instructions for Claude Code
├── README.md                         # This file
├── scripts/
│   ├── oi_install.sh                 # One-shot install + permissions check
│   ├── oi_screenshot.py              # Screen capture with Retina metadata
│   ├── oi_click.py                   # Mouse click by coordinates or OCR text
│   ├── oi_type.py                    # Keyboard input, hotkeys, key presses
│   ├── oi_find_text.py               # OCR: find text on screen → JSON coords
│   ├── oi_computer.py                # Unified dispatch for all actions
│   ├── oi_os_mode.py                 # Launch OI as managed subprocess
│   └── oi_permission_check.py        # Check macOS permissions
└── references/
    ├── computer-api.md               # OI Computer API reference
    ├── os-mode.md                    # OS Mode usage and architecture
    └── safety-and-permissions.md     # Permissions guide and safety model
```

## Scripts

### oi_screenshot.py — Screen capture

```bash
python3 scripts/oi_screenshot.py                          # Full screen
python3 scripts/oi_screenshot.py --region 0,0,800,600     # Region
python3 scripts/oi_screenshot.py --active-window           # Active window only
```

Outputs file path + `SCALE_FACTOR` + `SCREEN_SIZE` metadata (3 lines to stdout).

### oi_click.py — Mouse click

```bash
python3 scripts/oi_click.py --x 450 --y 300               # Coordinate click
python3 scripts/oi_click.py --x 900 --y 600 --image-coords # Auto-divide by Retina scale
python3 scripts/oi_click.py --text "Submit"                # OCR: find and click text
python3 scripts/oi_click.py --x 450 --y 300 --double      # Double click
python3 scripts/oi_click.py --x 450 --y 300 --right       # Right click
```

### oi_type.py — Keyboard input

```bash
python3 scripts/oi_type.py --text "hello world"            # Clipboard-paste (default)
python3 scripts/oi_type.py --key enter                     # Single key press
python3 scripts/oi_type.py --hotkey command space           # Hotkey (AppleScript on macOS)
python3 scripts/oi_type.py --text "search" --method typewrite  # Character-by-character
```

### oi_find_text.py — OCR screen reading

```bash
python3 scripts/oi_find_text.py --text "Submit"
python3 scripts/oi_find_text.py --text "Price" --all --min-conf 80
```

Returns JSON: `[{"text": "Submit", "x": 450, "y": 300, "w": 80, "h": 24, "confidence": 95}]`

### oi_computer.py — Unified dispatch

```bash
python3 scripts/oi_computer.py screenshot
python3 scripts/oi_computer.py click --x 450 --y 300
python3 scripts/oi_computer.py type --text "hello"
python3 scripts/oi_computer.py find --text "Submit"
python3 scripts/oi_computer.py scroll --clicks 3
python3 scripts/oi_computer.py mouse-position
python3 scripts/oi_computer.py screen-size
```

### oi_os_mode.py — Delegate full GUI tasks

```bash
python3 scripts/oi_os_mode.py "Open Calculator and compute 2+2"
python3 scripts/oi_os_mode.py --local "What apps are open?"      # Ollama (offline)
```

## Quick Examples

### Open an app via Spotlight

```bash
python3 scripts/oi_type.py --hotkey command space
sleep 0.5
python3 scripts/oi_type.py --text "Calculator"
sleep 0.3
python3 scripts/oi_type.py --key enter
```

### Click a button by label

```bash
python3 scripts/oi_click.py --text "Save"
```

### Read text from screen

```bash
python3 scripts/oi_find_text.py --text "Total" --all
```

### Fill a form

```bash
python3 scripts/oi_click.py --text "Email"
python3 scripts/oi_type.py --text "user@example.com"
python3 scripts/oi_type.py --key tab
python3 scripts/oi_type.py --text "password123"
```

## Retina Display Handling

macOS Retina displays render at 2x scaling. Screenshot image pixels differ from pyautogui screen coordinates. Use `--image-coords` on `oi_click.py` to auto-divide coordinates by the scale factor when targeting positions from screenshot pixels.

## Safety

1. Confirm with user before clicking Send, Delete, Submit, or Confirm buttons
2. Screenshot before and after every action for verification
3. No unbounded autonomous loops
4. pyautogui failsafe: moving mouse to screen corner raises exception
5. Every script logs actions to stderr: `[oi] click at (450, 300) button=left`

## Troubleshooting

| Symptom | Fix |
|---------|-----|
| Black screenshot | Grant Screen Recording permission to terminal app |
| Click/type no effect | Grant Accessibility permission to terminal app |
| OCR finds no text | Verify tesseract: `which tesseract && tesseract --version` |
| Coordinates off by 2x | Use `--image-coords` flag on `oi_click.py` |
| OS Mode hangs | Verify `ANTHROPIC_API_KEY` is set |
| Local mode fails | Verify Ollama running: `ollama list` |

## Credits

- [OpenInterpreter](https://github.com/OpenInterpreter/open-interpreter) by Killian Lucas — the foundation this skill builds on
- [Claudicle](https://github.com/tdimino/claudicle) by Tom di Mino — open-source soul agent framework, LLM-agnostic at the cognitive level
- Built as a [Claude Code skill](https://code.claude.com/docs/en/skills) following the [Agent Skills](https://agentskills.io/) open standard
