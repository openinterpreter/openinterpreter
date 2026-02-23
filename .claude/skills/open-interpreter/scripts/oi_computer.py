#!/usr/bin/env python3
"""
oi_computer.py -- Unified dispatch for all desktop automation actions.

Usage:
    oi_computer.py screenshot [--region X,Y,W,H] [--active-window]
    oi_computer.py click --x 450 --y 300 [--image-coords] [--double] [--right]
    oi_computer.py click --text "Submit"
    oi_computer.py type --text "hello world" [--method typewrite]
    oi_computer.py type --key enter
    oi_computer.py type --hotkey command space
    oi_computer.py find --text "Submit" [--all] [--min-conf 80]
    oi_computer.py scroll --clicks 3 [--x 450 --y 300]
    oi_computer.py mouse-position
    oi_computer.py screen-size
"""

import json
import os
import subprocess
import sys

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))


def run_script(name, args):
    """Run a sibling script and pass through its output."""
    script = os.path.join(SCRIPT_DIR, name)
    cmd = [sys.executable, script] + args
    result = subprocess.run(cmd, capture_output=False)
    return result.returncode


def main():
    if len(sys.argv) < 2:
        print("Usage: oi_computer.py <command> [args...]", file=sys.stderr)
        print("Commands: screenshot, click, type, find, scroll, mouse-position, screen-size", file=sys.stderr)
        sys.exit(1)

    command = sys.argv[1]
    args = sys.argv[2:]

    if command == "screenshot":
        sys.exit(run_script("oi_screenshot.py", args))

    elif command == "click":
        sys.exit(run_script("oi_click.py", args))

    elif command == "type":
        sys.exit(run_script("oi_type.py", args))

    elif command == "find":
        sys.exit(run_script("oi_find_text.py", args))

    elif command == "scroll":
        import argparse
        parser = argparse.ArgumentParser()
        parser.add_argument("--clicks", type=int, default=3, help="Scroll clicks (positive=up, negative=down)")
        parser.add_argument("--x", type=int, help="X position to scroll at")
        parser.add_argument("--y", type=int, help="Y position to scroll at")
        parsed = parser.parse_args(args)

        import pyautogui
        if parsed.x is not None and parsed.y is not None:
            pyautogui.moveTo(parsed.x, parsed.y)
        pyautogui.scroll(parsed.clicks)
        print(f"[oi] scroll clicks={parsed.clicks}", file=sys.stderr)
        print(json.dumps({"action": "scroll", "clicks": parsed.clicks}))

    elif command == "mouse-position":
        import pyautogui
        pos = pyautogui.position()
        print(json.dumps({"x": pos.x, "y": pos.y}))

    elif command == "screen-size":
        import pyautogui
        size = pyautogui.size()
        print(json.dumps({"width": size.width, "height": size.height}))

    else:
        print(f"Error: unknown command '{command}'", file=sys.stderr)
        print("Commands: screenshot, click, type, find, scroll, mouse-position, screen-size", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
