#!/usr/bin/env python3
"""
oi_screenshot.py -- Capture screen and return file path with Retina metadata.

Outputs 3 lines to stdout:
  1. File path to PNG screenshot
  2. SCALE_FACTOR=N (Retina multiplier)
  3. SCREEN_SIZE=WxH (pyautogui coordinates)

Usage:
    oi_screenshot.py                          # Full screen
    oi_screenshot.py --region 0,0,800,600     # Region (x,y,w,h)
    oi_screenshot.py --active-window          # Active window only (macOS)
    oi_screenshot.py --output /tmp/my.png     # Custom output path
"""

import argparse
import os
import platform
import subprocess
import sys
import tempfile
import time


def get_scale_factor():
    """Detect Retina scale factor on macOS."""
    if platform.system() != "Darwin":
        return 1

    try:
        # Use system_profiler to get display info
        result = subprocess.run(
            ["system_profiler", "SPDisplaysDataType"],
            capture_output=True, text=True, timeout=5
        )
        output = result.stdout
        # Look for Resolution line with Retina indicator
        for line in output.splitlines():
            if "Retina" in line or "Resolution" in line:
                if "Retina" in line:
                    return 2
        # Fallback: compare screenshot size to pyautogui screen size
        return _detect_scale_from_screenshot()
    except Exception:
        return _detect_scale_from_screenshot()


def _detect_scale_from_screenshot():
    """Detect scale factor by comparing screenshot dimensions to screen size."""
    try:
        import pyautogui
        screen = pyautogui.size()

        # Take a tiny test screenshot
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
    return 2  # Default assumption for modern Macs


def screenshot_macos(output_path, region=None, active_window=False):
    """Take screenshot on macOS using screencapture."""
    cmd = ["screencapture", "-x", "-C"]

    if active_window:
        # Get frontmost window ID via AppleScript
        try:
            result = subprocess.run(
                ["osascript", "-e",
                 'tell application "System Events" to get id of first window of (first process whose frontmost is true)'],
                capture_output=True, text=True, timeout=5
            )
            window_id = result.stdout.strip()
            if window_id and window_id.isdigit():
                cmd.extend(["-l", window_id])
            else:
                # Fallback to full-screen capture (never use -w which hangs in automation)
                print("[oi] warning: could not get window ID, falling back to full screen", file=sys.stderr)
        except Exception:
            print("[oi] warning: could not get window ID, falling back to full screen", file=sys.stderr)

    if region:
        cmd.extend(["-R", region])

    cmd.append(output_path)
    subprocess.run(cmd, capture_output=True, timeout=10)
    return os.path.exists(output_path)


def screenshot_linux(output_path, region=None, active_window=False):
    """Take screenshot on Linux using scrot or import."""
    for tool in ["scrot", "import"]:
        if subprocess.run(["which", tool], capture_output=True).returncode == 0:
            if tool == "scrot":
                cmd = ["scrot", output_path]
                if active_window:
                    cmd = ["scrot", "-u", output_path]
                elif region:
                    x, y, w, h = region.split(",")
                    cmd = ["scrot", "-a", f"{x},{y},{w},{h}", output_path]
            else:  # import (ImageMagick)
                cmd = ["import", "-window", "root", output_path]
                if active_window:
                    cmd = ["import", output_path]  # Interactive

            subprocess.run(cmd, capture_output=True, timeout=10)
            return os.path.exists(output_path)

    # Fallback: pyautogui
    try:
        import pyautogui
        img = pyautogui.screenshot(region=tuple(map(int, region.split(","))) if region else None)
        img.save(output_path)
        return True
    except Exception:
        return False


def main():
    parser = argparse.ArgumentParser(description="Capture screen, return path + Retina metadata")
    parser.add_argument("--region", metavar="X,Y,W,H", help="Capture region (x,y,width,height)")
    parser.add_argument("--active-window", action="store_true", help="Capture active window only")
    parser.add_argument("--output", "-o", metavar="PATH", help="Custom output file path")

    args = parser.parse_args()

    # Generate output path
    if args.output:
        output_path = args.output
    else:
        timestamp = int(time.time())
        output_path = os.path.join(tempfile.gettempdir(), f"oi_screenshot_{timestamp}.png")

    # Take screenshot
    system = platform.system()
    if system == "Darwin":
        ok = screenshot_macos(output_path, region=args.region, active_window=args.active_window)
    elif system == "Linux":
        ok = screenshot_linux(output_path, region=args.region, active_window=args.active_window)
    else:
        # Fallback: pyautogui
        try:
            import pyautogui
            region_tuple = tuple(map(int, args.region.split(","))) if args.region else None
            img = pyautogui.screenshot(region=region_tuple)
            img.save(output_path)
            ok = True
        except Exception as e:
            print(f"Error: {e}", file=sys.stderr)
            ok = False

    if not ok or not os.path.exists(output_path):
        print("Error: screenshot failed", file=sys.stderr)
        sys.exit(1)

    # Get metadata
    scale_factor = get_scale_factor()
    try:
        import pyautogui
        screen = pyautogui.size()
        screen_size = f"{screen.width}x{screen.height}"
    except Exception:
        screen_size = "unknown"

    # Output: path + metadata
    print(output_path)
    print(f"SCALE_FACTOR={scale_factor}")
    print(f"SCREEN_SIZE={screen_size}")

    print(f"[oi] screenshot saved: {output_path} (scale={scale_factor}, screen={screen_size})", file=sys.stderr)


if __name__ == "__main__":
    main()
