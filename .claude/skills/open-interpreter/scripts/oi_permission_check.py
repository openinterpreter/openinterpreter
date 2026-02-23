#!/usr/bin/env python3
"""
oi_permission_check.py -- Check macOS permissions for desktop GUI automation.

Verifies:
  1. Accessibility permission (required for pyautogui mouse/keyboard)
  2. Screen Recording permission (required for screenshots)
  3. tesseract installation (required for OCR)

Usage:
    python3 oi_permission_check.py
"""

import platform
import shutil
import subprocess
import sys


def check_accessibility():
    """Check if Accessibility permission is granted (macOS only)."""
    if platform.system() != "Darwin":
        print("  Accessibility: N/A (not macOS)")
        return True

    # Try a minimal pyautogui operation to detect permission
    try:
        import pyautogui
        # position() requires Accessibility on macOS
        pos = pyautogui.position()
        print(f"  Accessibility: OK (mouse at {pos.x}, {pos.y})")
        return True
    except Exception as e:
        err = str(e).lower()
        if "accessibility" in err or "permission" in err or "not allowed" in err:
            print("  Accessibility: DENIED")
            print("    -> System Settings > Privacy & Security > Accessibility > add your terminal app")
            return False
        # If the error is something else, pyautogui may still work
        print(f"  Accessibility: UNKNOWN ({e})")
        return True


def check_screen_recording():
    """Check if Screen Recording permission is granted (macOS only)."""
    if platform.system() != "Darwin":
        print("  Screen Recording: N/A (not macOS)")
        return True

    # Take a test screenshot with screencapture
    import tempfile
    import os
    tmp = os.path.join(tempfile.gettempdir(), "oi_perm_test.png")
    try:
        result = subprocess.run(
            ["screencapture", "-x", "-C", tmp],
            capture_output=True, timeout=5
        )
        if os.path.exists(tmp):
            size = os.path.getsize(tmp)
            os.unlink(tmp)
            if size > 100:
                print(f"  Screen Recording: OK (test screenshot {size} bytes)")
                return True
            else:
                print("  Screen Recording: DENIED (screenshot is empty)")
                print("    -> System Settings > Privacy & Security > Screen Recording > add your terminal app")
                return False
        else:
            print("  Screen Recording: DENIED (no screenshot produced)")
            print("    -> System Settings > Privacy & Security > Screen Recording > add your terminal app")
            return False
    except subprocess.TimeoutExpired:
        print("  Screen Recording: TIMEOUT (screencapture hung — permission dialog may be showing)")
        return False
    except FileNotFoundError:
        print("  Screen Recording: N/A (screencapture not found)")
        return True
    finally:
        if os.path.exists(tmp):
            os.unlink(tmp)


def check_tesseract():
    """Check if tesseract OCR is installed."""
    path = shutil.which("tesseract")
    if path:
        try:
            result = subprocess.run(
                ["tesseract", "--version"],
                capture_output=True, text=True, timeout=5
            )
            version = result.stdout.strip().split("\n")[0] if result.stdout else result.stderr.strip().split("\n")[0]
            print(f"  tesseract: OK ({version} at {path})")
            return True
        except Exception:
            print(f"  tesseract: OK (at {path}, version check failed)")
            return True
    else:
        print("  tesseract: NOT FOUND")
        print("    -> Install: brew install tesseract")
        return False


def check_pyautogui():
    """Check if pyautogui is installed."""
    try:
        import pyautogui
        size = pyautogui.size()
        print(f"  pyautogui: OK (screen: {size.width}x{size.height})")
        return True
    except ImportError:
        print("  pyautogui: NOT INSTALLED")
        print("    -> Run: ~/.claude/skills/open-interpreter/scripts/oi_install.sh")
        return False
    except Exception as e:
        print(f"  pyautogui: ERROR ({e})")
        return False


def main():
    print("OpenInterpreter Permission Check")
    print("=" * 40)

    all_ok = True
    all_ok &= check_pyautogui()
    all_ok &= check_accessibility()
    all_ok &= check_screen_recording()
    all_ok &= check_tesseract()

    print("=" * 40)
    if all_ok:
        print("All checks passed.")
    else:
        print("Some checks failed. See instructions above.")
        sys.exit(1)


if __name__ == "__main__":
    main()
