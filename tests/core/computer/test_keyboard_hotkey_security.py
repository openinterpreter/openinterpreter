import unittest
from unittest import mock

from interpreter.core.computer.keyboard.keyboard import Keyboard


class TestKeyboardHotkeySecurity(unittest.TestCase):
    def setUp(self):
        self.keyboard = Keyboard(mock.Mock())

    @mock.patch("interpreter.core.computer.keyboard.keyboard.time.sleep")
    @mock.patch("interpreter.core.computer.keyboard.keyboard.platform.system")
    @mock.patch("interpreter.core.computer.keyboard.keyboard.subprocess.run")
    def test_macos_hotkey_uses_subprocess_argument_list(
        self, mock_subprocess_run, mock_platform_system, _
    ):
        mock_platform_system.return_value = "Darwin"
        mock_pyautogui = mock.Mock()

        with mock.patch(
            "interpreter.core.computer.keyboard.keyboard.pyautogui", mock_pyautogui
        ):
            self.keyboard.hotkey("a", "command")

        mock_subprocess_run.assert_called_once()
        command = mock_subprocess_run.call_args[0][0]
        self.assertIsInstance(command, list)
        self.assertEqual(command[0], "osascript")
        self.assertEqual(command[1], "-e")
        self.assertIn('keystroke "a" using command down', command[2])
        mock_pyautogui.hotkey.assert_not_called()

    @mock.patch("interpreter.core.computer.keyboard.keyboard.time.sleep")
    @mock.patch("interpreter.core.computer.keyboard.keyboard.platform.system")
    @mock.patch("interpreter.core.computer.keyboard.keyboard.subprocess.run")
    def test_macos_hotkey_escapes_applescript_keystroke_content(
        self, mock_subprocess_run, mock_platform_system, _
    ):
        mock_platform_system.return_value = "Darwin"
        mock_pyautogui = mock.Mock()
        payload = 'a"b\\c'

        with mock.patch(
            "interpreter.core.computer.keyboard.keyboard.pyautogui", mock_pyautogui
        ):
            self.keyboard.hotkey(payload, "command")

        command = mock_subprocess_run.call_args[0][0]
        self.assertIn('keystroke "a\\"b\\\\c" using command down', command[2])
        mock_pyautogui.hotkey.assert_not_called()

    @mock.patch("interpreter.core.computer.keyboard.keyboard.time.sleep")
    @mock.patch("interpreter.core.computer.keyboard.keyboard.platform.system")
    @mock.patch("interpreter.core.computer.keyboard.keyboard.subprocess.run")
    def test_invalid_modifier_falls_back_to_pyautogui(
        self, mock_subprocess_run, mock_platform_system, _
    ):
        mock_platform_system.return_value = "Darwin"
        mock_pyautogui = mock.Mock()

        with mock.patch(
            "interpreter.core.computer.keyboard.keyboard.pyautogui", mock_pyautogui
        ):
            self.keyboard.hotkey("a", "not_a_modifier")

        mock_subprocess_run.assert_not_called()
        mock_pyautogui.hotkey.assert_called_once_with(
            "a", "not_a_modifier", interval=0.1
        )

    @mock.patch("interpreter.core.computer.keyboard.keyboard.time.sleep")
    @mock.patch("interpreter.core.computer.keyboard.keyboard.platform.system")
    @mock.patch("interpreter.core.computer.keyboard.keyboard.subprocess.run")
    def test_non_macos_uses_pyautogui_hotkey(
        self, mock_subprocess_run, mock_platform_system, _
    ):
        mock_platform_system.return_value = "Linux"
        mock_pyautogui = mock.Mock()

        with mock.patch(
            "interpreter.core.computer.keyboard.keyboard.pyautogui", mock_pyautogui
        ):
            self.keyboard.hotkey("ctrl", "x", interval=0.2)

        mock_subprocess_run.assert_not_called()
        mock_pyautogui.hotkey.assert_called_once_with("ctrl", "x", interval=0.2)


if __name__ == "__main__":
    unittest.main()
