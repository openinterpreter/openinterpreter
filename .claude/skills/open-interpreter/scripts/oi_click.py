#!/usr/bin/env python3
"""
oi_click.py -- Mouse click by coordinates or OCR text.

Usage:
    oi_click.py --x 450 --y 300              # Click at screen coordinates
    oi_click.py --x 900 --y 600 --image-coords  # Auto-divide by Retina scale
    oi_click.py --text "Submit"              # OCR: find text on screen, click center
    oi_click.py --x 450 --y 300 --double    # Double click
    oi_click.py --x 450 --y 300 --right     # Right click
    oi_click.py --x 450 --y 300 --clicks 3  # Triple click
"""

import argparse
import json
import os
import platform
import subprocess
import sys
import tempfile
import time


def get_scale_factor():
    """Detect Retina scale factor."""
    if platform.system() != "Darwin":
        return 1
    try:
        import pyautogui
        screen = pyautogui.size()
        tmp = os.path.join(tempfile.gettempdir(), "oi_scale_test.png")
        subprocess.run(["screencapture", "-x", "-C", tmp], capture_output=True, timeout=5)
        if os.path.exists(tmp):
            from PIL import Image
            img = Image.open(tmp)
            img_w = img.width
            img.close()
            os.unlink(tmp)
            factor = round(img_w / screen.width)
            return max(1, factor)
    except Exception:
        pass
    return 2


def find_text_on_screen(text):
    """Find text on screen using pytesseract OCR. Returns (x, y) center coordinates."""
    import pyautogui
    import pytesseract
    from PIL import Image

    # Take screenshot
    tmp = os.path.join(tempfile.gettempdir(), f"oi_ocr_{int(time.time())}.png")
    if platform.system() == "Darwin":
        subprocess.run(["screencapture", "-x", "-C", tmp], capture_output=True, timeout=5)
    else:
        img = pyautogui.screenshot()
        img.save(tmp)

    if not os.path.exists(tmp):
        return None

    img = Image.open(tmp)
    scale = get_scale_factor()

    # Run OCR with bounding box data
    data = pytesseract.image_to_data(img, output_type=pytesseract.Output.DICT)
    img.close()
    os.unlink(tmp)

    # Search for matching text
    best_match = None
    best_conf = -1
    text_lower = text.lower()

    n = len(data["text"])
    for i in range(n):
        word = data["text"][i].strip()
        if not word:
            continue
        conf = int(data["conf"][i]) if data["conf"][i] != "-1" else 0
        if text_lower in word.lower() and conf > best_conf:
            # Center of bounding box in image space, then convert to screen coordinates
            x = int((data["left"][i] + data["width"][i] / 2) / scale)
            y = int((data["top"][i] + data["height"][i] / 2) / scale)
            best_match = (x, y)
            best_conf = conf

    # Also try matching across consecutive words
    if best_match is None and " " in text:
        words = text_lower.split()
        for i in range(n - len(words) + 1):
            segment = " ".join(data["text"][i:i + len(words)]).strip().lower()
            if text_lower in segment:
                # Span from first to last word
                x1 = data["left"][i]
                y1 = data["top"][i]
                last = i + len(words) - 1
                x2 = data["left"][last] + data["width"][last]
                y2 = data["top"][last] + data["height"][last]
                x = int((x1 + x2) / 2 / scale)
                y = int((y1 + y2) / 2 / scale)
                best_match = (x, y)
                break

    return best_match


def main():
    parser = argparse.ArgumentParser(description="Click at coordinates or OCR text location")
    parser.add_argument("--x", type=int, help="X coordinate")
    parser.add_argument("--y", type=int, help="Y coordinate")
    parser.add_argument("--text", metavar="TEXT", help="Find text on screen via OCR and click its center")
    parser.add_argument("--image-coords", action="store_true",
                        help="Divide coordinates by Retina scale factor (use when coords come from screenshot pixels)")
    parser.add_argument("--double", action="store_true", help="Double click")
    parser.add_argument("--right", action="store_true", help="Right click")
    parser.add_argument("--clicks", type=int, default=1, help="Number of clicks (default: 1)")

    args = parser.parse_args()

    if not args.text and (args.x is None or args.y is None):
        parser.error("Provide either --text or both --x and --y")

    try:
        import pyautogui
    except ImportError:
        print("Error: pyautogui not installed. Run: ~/.claude/skills/open-interpreter/scripts/oi_install.sh",
              file=sys.stderr)
        sys.exit(1)

    if args.text:
        # OCR mode
        result = find_text_on_screen(args.text)
        if result is None:
            print(f"Error: text '{args.text}' not found on screen", file=sys.stderr)
            sys.exit(1)
        x, y = result
        print(f"[oi] found '{args.text}' at ({x}, {y})", file=sys.stderr)
    else:
        x, y = args.x, args.y
        if args.image_coords:
            scale = get_scale_factor()
            x = x // scale
            y = y // scale
            print(f"[oi] image coords ({args.x}, {args.y}) -> screen coords ({x}, {y}) (scale={scale})",
                  file=sys.stderr)

    # Perform click
    button = "right" if args.right else "left"
    clicks = args.clicks if args.clicks > 1 else (2 if args.double else 1)

    pyautogui.click(x, y, clicks=clicks, button=button)
    print(f"[oi] click at ({x}, {y}) button={button} clicks={clicks}", file=sys.stderr)
    print(json.dumps({"action": "click", "x": x, "y": y, "button": button, "clicks": clicks}))


if __name__ == "__main__":
    main()
