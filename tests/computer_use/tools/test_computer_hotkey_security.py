import importlib
import sys
import unittest
from unittest import mock


def load_computer_module_with_mock_pyautogui():
    module_name = "interpreter.computer_use.tools.computer"
    sys.modules.pop(module_name, None)
    sys.modules.pop("interpreter.computer_use.tools", None)

    mock_pyautogui = mock.Mock()
    mock_pyautogui.size.return_value = (1920, 1080)

    with mock.patch.dict(sys.modules, {"pyautogui": mock_pyautogui}):
        module = importlib.import_module(module_name)

    return module, mock_pyautogui


class TestComputerToolHotkeySecurity(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.computer_module, self.mock_pyautogui = (
            load_computer_module_with_mock_pyautogui()
        )

    async def test_macos_key_action_uses_hotkey_runner(self):
        tool = self.computer_module.ComputerTool()
        tool.screenshot = mock.AsyncMock(
            return_value=self.computer_module.ToolResult(base64_image="ok")
        )
        with mock.patch.object(
            self.computer_module.platform, "system", return_value="Darwin"
        ), mock.patch.object(
            self.computer_module, "_run_macos_hotkey", return_value=True
        ) as mock_run_macos_hotkey:
            await tool(action="key", text="command+a")

        mock_run_macos_hotkey.assert_called_once_with("a", ["command"])
        self.mock_pyautogui.hotkey.assert_not_called()

    async def test_macos_key_action_falls_back_to_pyautogui_hotkey(self):
        tool = self.computer_module.ComputerTool()
        tool.screenshot = mock.AsyncMock(
            return_value=self.computer_module.ToolResult(base64_image="ok")
        )
        with mock.patch.object(
            self.computer_module.platform, "system", return_value="Darwin"
        ), mock.patch.object(
            self.computer_module, "_run_macos_hotkey", return_value=False
        ) as mock_run_macos_hotkey:
            await tool(action="key", text="bad+a")

        mock_run_macos_hotkey.assert_called_once_with("a", ["bad"])
        self.mock_pyautogui.hotkey.assert_called_once_with("bad", "a")

    def test_run_macos_hotkey_escapes_and_uses_subprocess_list_args(self):
        with mock.patch.object(self.computer_module.subprocess, "run") as mock_run:
            success = self.computer_module._run_macos_hotkey('a"b\\c', ["command"])

        self.assertTrue(success)
        mock_run.assert_called_once()
        command = mock_run.call_args[0][0]
        self.assertIsInstance(command, list)
        self.assertEqual(command[0], "osascript")
        self.assertEqual(command[1], "-e")
        self.assertIn('keystroke "a\\"b\\\\c" using command down', command[2])

    def test_run_macos_hotkey_rejects_invalid_modifier(self):
        with mock.patch.object(self.computer_module.subprocess, "run") as mock_run:
            success = self.computer_module._run_macos_hotkey("a", ["not_a_modifier"])

        self.assertFalse(success)
        mock_run.assert_not_called()


if __name__ == "__main__":
    unittest.main()
