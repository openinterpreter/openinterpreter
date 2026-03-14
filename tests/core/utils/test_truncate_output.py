import importlib.util
import tempfile
import unittest
from pathlib import Path
from unittest import mock


MODULE_PATH = Path(__file__).resolve().parents[3] / "interpreter/core/utils/truncate_output.py"
SPEC = importlib.util.spec_from_file_location("truncate_output_module", MODULE_PATH)
truncate_output_module = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(truncate_output_module)
truncate_output = truncate_output_module.truncate_output


class TestTruncateOutput(unittest.TestCase):
    def test_does_not_truncate_when_only_ansi_codes_exceed_limit(self):
        data = "\x1b[31mhello\x1b[0m"

        result = truncate_output(data, max_output_chars=5)

        self.assertEqual(result, data)

    def test_truncation_message_uses_real_recovery_paths(self):
        data = "abcdefghijklmnopqrstuvwxyz"

        with tempfile.TemporaryDirectory() as temp_dir:
            with mock.patch.object(
                truncate_output_module.tempfile,
                "gettempdir",
                return_value=temp_dir,
            ):
                result = truncate_output(data, max_output_chars=10)

            saved_output = Path(temp_dir) / "oi-output-latest.txt"
            self.assertTrue(saved_output.exists())
            self.assertEqual(saved_output.read_text(), data)

        self.assertIn("head", result)
        self.assertIn("tail", result)
        self.assertIn("grep", result)
        self.assertNotIn("computer.ai.summarize(result)", result)

