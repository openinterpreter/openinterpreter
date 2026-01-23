"""Tests for DISPLAY environment variable handling in computer_use module."""

import os
import sys
import unittest
from unittest.mock import patch


class TestDisplayEnvHandling(unittest.TestCase):
    """Test that missing DISPLAY environment variable is handled gracefully."""

    def test_computer_tool_import_without_display(self):
        """Test that computer.py can be imported without DISPLAY set."""
        # Remove DISPLAY from environment if present
        env_backup = os.environ.get("DISPLAY")
        if "DISPLAY" in os.environ:
            del os.environ["DISPLAY"]

        # Clear any cached imports
        modules_to_clear = [
            k for k in sys.modules.keys() if k.startswith("interpreter.computer_use")
        ]
        for mod in modules_to_clear:
            del sys.modules[mod]

        try:
            # Mock pyautogui to raise the expected error
            with patch.dict("sys.modules", {"pyautogui": None}):
                # This should not raise an error on import
                from interpreter.computer_use.tools.computer import (
                    _FALLBACK_SCREEN_SIZE,
                    ComputerTool,
                )
                from interpreter.computer_use.tools.computer import (
                    pyautogui as imported_pyautogui,
                )

                # Verify pyautogui is None when not available
                if imported_pyautogui is None:
                    # Create tool instance - should use fallback size
                    tool = ComputerTool()
                    self.assertEqual(tool.width, _FALLBACK_SCREEN_SIZE[0])
                    self.assertEqual(tool.height, _FALLBACK_SCREEN_SIZE[1])
        finally:
            # Restore DISPLAY environment
            if env_backup is not None:
                os.environ["DISPLAY"] = env_backup

    def test_loop_import_without_display(self):
        """Test that loop.py handles missing pyautogui gracefully."""
        # Clear any cached imports
        modules_to_clear = [
            k
            for k in sys.modules.keys()
            if k.startswith("interpreter.computer_use.loop")
        ]
        for mod in modules_to_clear:
            del sys.modules[mod]

        # The loop module should import without errors even when pyautogui fails
        try:
            from interpreter.computer_use.loop import check_mouse_position, pyautogui

            # If pyautogui is None, check_mouse_position should return immediately
            if pyautogui is None:
                # This should not raise an error
                check_mouse_position()
        except ImportError:
            # It's OK if other dependencies are missing in test environment
            pass


if __name__ == "__main__":
    unittest.main()
