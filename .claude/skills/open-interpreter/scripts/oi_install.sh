#!/usr/bin/env bash
# oi_install.sh -- One-shot OpenInterpreter installation and verification
#
# Installs open-interpreter[os] via uv, verifies pyautogui, tesseract,
# and checks macOS permissions.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

echo "==> Installing OpenInterpreter with OS mode extras..."
# OI pins tiktoken==0.7.0 which has no prebuilt wheel for Python 3.13+.
# Use --override to force a newer tiktoken that ships prebuilt wheels.
OVERRIDE_FILE=$(mktemp)
echo "tiktoken>=0.8" > "$OVERRIDE_FILE"
uv pip install --system "open-interpreter[os]" --override "$OVERRIDE_FILE"
rm -f "$OVERRIDE_FILE"

echo ""
echo "==> Verifying OpenInterpreter import..."
python3 -c "from interpreter import interpreter; print('  interpreter: OK')"

echo ""
echo "==> Verifying pyautogui..."
python3 -c "
import pyautogui
size = pyautogui.size()
print(f'  pyautogui: OK (screen: {size.width}x{size.height})')
"

echo ""
echo "==> Verifying pytesseract..."
python3 -c "
import pytesseract
version = pytesseract.get_tesseract_version()
print(f'  pytesseract: OK (tesseract {version})')
" 2>/dev/null || {
    echo "  pytesseract: MISSING"
    echo "  Install tesseract: brew install tesseract"
}

echo ""
echo "==> Checking tesseract CLI..."
if command -v tesseract &>/dev/null; then
    echo "  tesseract: $(tesseract --version 2>&1 | head -1)"
else
    echo "  tesseract: NOT FOUND"
    echo "  Install: brew install tesseract"
fi

echo ""
echo "==> Checking macOS permissions..."
python3 "$SCRIPT_DIR/oi_permission_check.py"

echo ""
echo "OpenInterpreter installation complete."
echo "If permissions are missing, add your terminal app in:"
echo "  System Settings > Privacy & Security > Accessibility"
echo "  System Settings > Privacy & Security > Screen Recording"
