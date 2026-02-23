#!/usr/bin/env python3
"""
oi_find_text.py -- OCR screen reading: find text locations on screen.

Returns JSON array of matches with coordinates (in screen space, not image pixels).

Usage:
    oi_find_text.py --text "Submit"
    oi_find_text.py --text "Submit" --screenshot /tmp/screenshot.png
    oi_find_text.py --text "Price" --all          # Return all matches, not just best
    oi_find_text.py --text "File" --min-conf 80   # Minimum confidence threshold
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


def find_text(text, screenshot_path=None, return_all=False, min_conf=0):
    """Find text on screen using pytesseract OCR.

    Returns list of dicts: [{"text": str, "x": int, "y": int, "w": int, "h": int, "confidence": int}]
    Coordinates are in screen space (divided by Retina scale factor).
    """
    import pytesseract
    from PIL import Image

    # Take screenshot if not provided
    tmp_created = False
    if screenshot_path is None:
        screenshot_path = os.path.join(tempfile.gettempdir(), f"oi_ocr_{int(time.time())}.png")
        if platform.system() == "Darwin":
            subprocess.run(["screencapture", "-x", "-C", screenshot_path], capture_output=True, timeout=5)
        else:
            import pyautogui
            img = pyautogui.screenshot()
            img.save(screenshot_path)
        tmp_created = True

    if not os.path.exists(screenshot_path):
        return []

    img = Image.open(screenshot_path)
    scale = get_scale_factor()

    # Run OCR
    data = pytesseract.image_to_data(img, output_type=pytesseract.Output.DICT)
    img.close()

    if tmp_created:
        os.unlink(screenshot_path)

    # Find matches
    matches = []
    text_lower = text.lower()
    n = len(data["text"])

    # Single-word matches
    for i in range(n):
        word = data["text"][i].strip()
        if not word:
            continue
        conf = int(data["conf"][i]) if data["conf"][i] != "-1" else 0
        if conf < min_conf:
            continue
        if text_lower in word.lower():
            matches.append({
                "text": word,
                "x": int((data["left"][i] + data["width"][i] / 2) / scale),
                "y": int((data["top"][i] + data["height"][i] / 2) / scale),
                "w": int(data["width"][i] / scale),
                "h": int(data["height"][i] / scale),
                "confidence": conf,
            })

    # Multi-word matches
    if " " in text:
        words = text_lower.split()
        for i in range(n - len(words) + 1):
            segment = " ".join(data["text"][i:i + len(words)]).strip().lower()
            if text_lower in segment:
                # Check minimum confidence across span
                span_conf = min(
                    int(data["conf"][j]) if data["conf"][j] != "-1" else 0
                    for j in range(i, i + len(words))
                )
                if span_conf < min_conf:
                    continue

                last = i + len(words) - 1
                x1 = data["left"][i]
                y1 = data["top"][i]
                x2 = data["left"][last] + data["width"][last]
                y2 = data["top"][last] + data["height"][last]

                matches.append({
                    "text": " ".join(data["text"][i:i + len(words)]),
                    "x": int((x1 + x2) / 2 / scale),
                    "y": int((y1 + y2) / 2 / scale),
                    "w": int((x2 - x1) / scale),
                    "h": int((y2 - y1) / scale),
                    "confidence": span_conf,
                })

    # Sort by confidence descending
    matches.sort(key=lambda m: m["confidence"], reverse=True)

    if return_all:
        return matches
    elif matches:
        return [matches[0]]
    else:
        return []


def main():
    parser = argparse.ArgumentParser(description="Find text on screen via OCR")
    parser.add_argument("--text", required=True, help="Text to search for")
    parser.add_argument("--screenshot", metavar="PATH", help="Use existing screenshot instead of capturing")
    parser.add_argument("--all", action="store_true", help="Return all matches, not just the best")
    parser.add_argument("--min-conf", type=int, default=0, help="Minimum OCR confidence threshold (0-100)")

    args = parser.parse_args()

    try:
        results = find_text(
            args.text,
            screenshot_path=args.screenshot,
            return_all=args.all,
            min_conf=args.min_conf,
        )

        print(json.dumps(results, indent=2))

        if results:
            print(f"[oi] found {len(results)} match(es) for '{args.text}'", file=sys.stderr)
        else:
            print(f"[oi] no matches for '{args.text}'", file=sys.stderr)
            sys.exit(1)

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
