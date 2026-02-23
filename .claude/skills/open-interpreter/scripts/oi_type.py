#!/usr/bin/env python3
"""
oi_type.py -- Keyboard input: text, keys, and hotkeys.

Usage:
    oi_type.py --text "hello world"              # Clipboard-paste (fast, unicode-safe)
    oi_type.py --text "search" --method typewrite # Character-by-character
    oi_type.py --key enter                       # Single key press
    oi_type.py --key tab                         # Tab key
    oi_type.py --hotkey command space             # Hotkey combo (AppleScript on macOS)
    oi_type.py --hotkey command shift 3           # Multi-modifier hotkey
"""

import argparse
import json
import platform
import subprocess
import sys


def paste_text(text):
    """Type text via clipboard-paste (Cmd+V on macOS, Ctrl+V elsewhere).
    Faster and Unicode-safe compared to character-by-character typing."""
    import pyperclip
    import pyautogui

    # Save current clipboard
    try:
        old_clipboard = pyperclip.paste()
    except Exception:
        old_clipboard = None

    # Copy text to clipboard and paste
    pyperclip.copy(text)
    if platform.system() == "Darwin":
        pyautogui.hotkey("command", "v")
    else:
        pyautogui.hotkey("ctrl", "v")

    # Restore clipboard after brief delay
    import time
    time.sleep(0.1)
    if old_clipboard is not None:
        try:
            pyperclip.copy(old_clipboard)
        except Exception:
            pass


def typewrite_text(text, interval=0.02):
    """Type text character-by-character. Slower but doesn't use clipboard."""
    import pyautogui
    pyautogui.typewrite(text, interval=interval)


def press_key(key):
    """Press a single key."""
    import pyautogui
    pyautogui.press(key)


def hotkey_applescript(*keys):
    """Execute hotkey via AppleScript (macOS). More reliable for modifier keys."""
    # Map key names to AppleScript key codes
    modifier_map = {
        "command": "command down",
        "cmd": "command down",
        "shift": "shift down",
        "option": "option down",
        "alt": "option down",
        "control": "control down",
        "ctrl": "control down",
    }

    modifiers = []
    key_char = None

    for k in keys:
        k_lower = k.lower()
        if k_lower in modifier_map:
            modifiers.append(modifier_map[k_lower])
        else:
            key_char = k_lower

    if key_char is None:
        # All modifiers, no key — just press the last modifier as a key
        key_char = keys[-1].lower()
        modifiers = modifiers[:-1]

    # Map special key names to AppleScript
    key_code_map = {
        "space": 49, "return": 36, "enter": 36, "tab": 48,
        "escape": 53, "esc": 53, "delete": 51, "backspace": 51,
        "up": 126, "down": 125, "left": 123, "right": 124,
        "f1": 122, "f2": 120, "f3": 99, "f4": 118,
        "f5": 96, "f6": 97, "f7": 98, "f8": 100,
    }

    modifier_str = ", ".join(modifiers) if modifiers else ""

    if key_char in key_code_map:
        code = key_code_map[key_char]
        if modifier_str:
            script = f'tell application "System Events" to key code {code} using {{{modifier_str}}}'
        else:
            script = f'tell application "System Events" to key code {code}'
    else:
        # Single character — sanitize to prevent AppleScript injection
        if len(key_char) != 1 or key_char in ('"', '\\'):
            print(f"Error: invalid key character for keystroke: {repr(key_char)}", file=sys.stderr)
            sys.exit(1)
        if modifier_str:
            script = f'tell application "System Events" to keystroke "{key_char}" using {{{modifier_str}}}'
        else:
            script = f'tell application "System Events" to keystroke "{key_char}"'

    subprocess.run(["osascript", "-e", script], capture_output=True, timeout=5)


def hotkey_pyautogui(*keys):
    """Execute hotkey via pyautogui (cross-platform fallback)."""
    import pyautogui
    pyautogui.hotkey(*keys)


def main():
    parser = argparse.ArgumentParser(description="Keyboard input: text, keys, and hotkeys")
    parser.add_argument("--text", metavar="TEXT", help="Text to type")
    parser.add_argument("--key", metavar="KEY", help="Single key to press (enter, tab, escape, etc.)")
    parser.add_argument("--hotkey", nargs="+", metavar="KEY", help="Hotkey combination (e.g., command space)")
    parser.add_argument("--method", choices=["paste", "typewrite"], default="paste",
                        help="Text input method: paste (clipboard, default) or typewrite (character-by-character)")

    args = parser.parse_args()

    if not any([args.text, args.key, args.hotkey]):
        parser.error("Provide one of --text, --key, or --hotkey")

    try:
        if args.text:
            if args.method == "typewrite":
                typewrite_text(args.text)
                print(f"[oi] typewrite: {repr(args.text)}", file=sys.stderr)
            else:
                paste_text(args.text)
                print(f"[oi] paste: {repr(args.text)}", file=sys.stderr)
            print(json.dumps({"action": "type", "text": args.text, "method": args.method}))

        elif args.key:
            import pyautogui
            pyautogui.press(args.key)
            print(f"[oi] key: {args.key}", file=sys.stderr)
            print(json.dumps({"action": "key", "key": args.key}))

        elif args.hotkey:
            if platform.system() == "Darwin":
                hotkey_applescript(*args.hotkey)
            else:
                hotkey_pyautogui(*args.hotkey)
            combo = "+".join(args.hotkey)
            print(f"[oi] hotkey: {combo}", file=sys.stderr)
            print(json.dumps({"action": "hotkey", "keys": args.hotkey}))

    except ImportError as e:
        print(f"Error: Missing dependency: {e}\n"
              "Run: ~/.claude/skills/open-interpreter/scripts/oi_install.sh",
              file=sys.stderr)
        sys.exit(1)
    except KeyboardInterrupt:
        sys.exit(130)
    except Exception as e:
        print(f"Error: {e}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
