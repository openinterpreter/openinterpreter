# OpenInterpreter Computer API Reference

## Overview

OpenInterpreter's Computer API (`interpreter/core/computer/`) provides programmatic access to desktop automation primitives. The skill wraps these via standalone scripts for use with Claude Code.

## Script Reference

### oi_screenshot.py

Captures the screen using `screencapture` (macOS) or `scrot`/pyautogui (Linux).

| Flag | Description |
|------|-------------|
| `--region X,Y,W,H` | Capture region only |
| `--active-window` | Capture frontmost window |
| `--output PATH` | Custom output path (default: `/tmp/oi_screenshot_TIMESTAMP.png`) |

**Output** (3 lines to stdout):
```
/tmp/oi_screenshot_1708789200.png
SCALE_FACTOR=2
SCREEN_SIZE=1512x982
```

### oi_click.py

Performs mouse clicks via pyautogui. Two modes: coordinate and OCR text.

| Flag | Description |
|------|-------------|
| `--x N --y N` | Click at screen coordinates |
| `--text "label"` | Find text via OCR, click center |
| `--image-coords` | Divide coords by Retina scale factor |
| `--double` | Double click |
| `--right` | Right click |
| `--clicks N` | Number of clicks (default: 1) |

**Output**: JSON object to stdout, action log to stderr.

### oi_type.py

Keyboard input: text, single keys, and hotkey combos.

| Flag | Description |
|------|-------------|
| `--text "string"` | Type text (default: clipboard paste) |
| `--key NAME` | Press single key (enter, tab, escape, etc.) |
| `--hotkey KEY KEY...` | Hotkey combo (e.g., command space) |
| `--method paste\|typewrite` | Text input method (default: paste) |

**Text methods**:
- `paste` (default): Copy to clipboard, Cmd+V. Fast, Unicode-safe.
- `typewrite`: Character-by-character. Slower, but doesn't touch clipboard.

**macOS hotkeys**: Uses AppleScript (`osascript`) for reliable modifier key handling. Key names: command, shift, option, control, plus key codes for special keys (space, enter, tab, escape, F1-F8, arrow keys).

### oi_find_text.py

OCR screen reading via pytesseract.

| Flag | Description |
|------|-------------|
| `--text "string"` | Text to search for (required) |
| `--screenshot PATH` | Use existing screenshot |
| `--all` | Return all matches, not just best |
| `--min-conf N` | Minimum confidence threshold (0-100) |

**Output**: JSON array to stdout:
```json
[{"text": "Submit", "x": 450, "y": 300, "w": 80, "h": 24, "confidence": 95}]
```

Coordinates are in screen space (divided by Retina scale).

### oi_computer.py

Unified dispatch. Routes to the appropriate script.

| Subcommand | Equivalent |
|------------|------------|
| `screenshot [args]` | `oi_screenshot.py [args]` |
| `click [args]` | `oi_click.py [args]` |
| `type [args]` | `oi_type.py [args]` |
| `find [args]` | `oi_find_text.py [args]` |
| `scroll --clicks N` | pyautogui.scroll() |
| `mouse-position` | Returns `{"x": N, "y": N}` |
| `screen-size` | Returns `{"width": N, "height": N}` |

## Retina Coordinate Handling

On macOS Retina displays, screenshot image pixels differ from pyautogui screen coordinates:

| | Image Pixels | Screen Coordinates |
|--|-------------|-------------------|
| 14" MBP | 3024 x 1964 | 1512 x 982 |
| Scale factor | 2x | 1x (pyautogui native) |

When Claude reads a screenshot and estimates a click target at pixel (900, 600) in the image:
- Without `--image-coords`: clicks at screen position (900, 600) — wrong
- With `--image-coords`: divides by 2, clicks at screen position (450, 300) — correct

The `oi_screenshot.py` SCALE_FACTOR output enables this conversion.

## Dependencies

| Package | Purpose | Install |
|---------|---------|---------|
| pyautogui | Mouse/keyboard control | `uv pip install pyautogui` |
| pytesseract | OCR text detection | `uv pip install pytesseract` |
| Pillow | Image processing | `uv pip install Pillow` |
| pyperclip | Clipboard access | `uv pip install pyperclip` |
| tesseract | OCR engine (CLI) | `brew install tesseract` |

All installed by `oi_install.sh`.
