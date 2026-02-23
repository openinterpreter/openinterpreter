# OpenInterpreter OS Mode Reference

## Overview

OS Mode (`interpreter --os`) is OpenInterpreter's screenshot-driven desktop control system. It runs an autonomous loop: screenshot → Claude API analysis → pyautogui action → repeat.

## Architecture

```
User task → OI agent loop:
  1. Take screenshot (screencapture / pyautogui)
  2. Send screenshot to Claude API (vision)
  3. Claude analyzes: what to do next?
  4. OI executes: click(x,y) / type("text") / hotkey(cmd+space)
  5. Take verification screenshot
  6. Repeat until task complete or max iterations
```

## Usage via oi_os_mode.py

```bash
# Default: Claude API via Anthropic
python3 scripts/oi_os_mode.py "Open Calculator and compute 2+2"

# Explicit provider
python3 scripts/oi_os_mode.py --provider anthropic "Change wallpaper"

# Custom timeout (default: 300s)
python3 scripts/oi_os_mode.py --timeout 120 "Fill out the form"
```

**Requirements**:
- `ANTHROPIC_API_KEY` environment variable
- macOS Accessibility + Screen Recording permissions

## Local Mode via Ollama

```bash
# Local model (code-execution mode, not screenshot-driven)
python3 scripts/oi_os_mode.py --local "What apps are open?"

# Custom model
python3 scripts/oi_os_mode.py --local --model llama3.2-vision "Describe the screen"

# Custom Ollama endpoint
python3 scripts/oi_os_mode.py --local --api-base http://192.168.1.100:11434 "List files"
```

**Requirements**:
- Ollama running: `ollama serve`
- Vision model: `ollama pull llama3.2-vision`

**Limitation**: Local models use OI's classic code-execution mode (generates Python/Bash to accomplish tasks). The screenshot-driven OS Mode is hardcoded to Claude 3.5 Sonnet and cannot use local models.

## When to Use Each Mode

| Scenario | Mode |
|----------|------|
| Precise GUI action (one click, one type) | Library (oi_click.py, oi_type.py) |
| Multi-step GUI workflow Claude can reason about | Library with screenshot loop |
| Self-contained GUI task, no codebase context | OS subprocess (oi_os_mode.py) |
| Offline / no API costs / privacy | Local (oi_os_mode.py --local) |
| Complex multi-app workflow | OS subprocess |

## OI CLI Flags Reference

| Flag | Description |
|------|-------------|
| `--os` | Enable OS Mode (screenshot-driven) |
| `-y` | Auto-approve actions (skip confirmation prompts) |
| `--model NAME` | LLM model to use |
| `--api_base URL` | Custom API endpoint |
| `--local` | Use local model (bundled profile) |
| `--vision` | Enable vision capabilities |
| `--safe_mode ask\|auto\|off` | Safety confirmation level |

## OI Python API (for advanced integration)

```python
from interpreter import interpreter

# OS Mode
interpreter.computer.display.screenshot()  # Take screenshot
interpreter.computer.mouse.click(x, y)     # Click at coordinates
interpreter.computer.keyboard.write("text") # Type text
interpreter.computer.keyboard.hotkey("command", "space")  # Hotkey

# Classic mode with Ollama
interpreter.llm.model = "ollama/llama3.2-vision"
interpreter.llm.api_base = "http://localhost:11434"
interpreter.auto_run = True
interpreter.chat("What time is it?")
```

## Development Status

OpenInterpreter is in maintenance mode (last release v0.4.2, Oct 2024). The core team pivoted to the 01 App. The codebase is stable and the underlying primitives (pyautogui, pytesseract) are well-maintained independently.

OS Mode is labeled "highly experimental" in OI's documentation. For production use, prefer Library mode (Claude Code reasons, scripts execute) over the OS subprocess approach.
