import unittest
from unittest import mock

from interpreter.core.computer.terminal.languages.shell import Shell
from interpreter.core.computer.terminal.terminal import Terminal


class TestTerminalGetLanguage(unittest.TestCase):
    def setUp(self):
        self.terminal = Terminal(mock.Mock())

    def test_cmd_maps_to_shell(self):
        # "cmd" is what models commonly emit for the Windows command prompt.
        # The Shell language already launches cmd.exe on Windows, so "cmd"
        # should resolve to it instead of being reported as unsupported.
        self.assertIs(self.terminal.get_language("cmd"), Shell)

    def test_cmd_is_case_insensitive(self):
        self.assertIs(self.terminal.get_language("CMD"), Shell)

    def test_existing_shell_aliases_still_resolve(self):
        for alias in ("bash", "sh", "zsh", "batch", "bat"):
            self.assertIs(self.terminal.get_language(alias), Shell)


if __name__ == "__main__":
    unittest.main()
