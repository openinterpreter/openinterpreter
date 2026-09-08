import os
import sys
from pathlib import Path
import tempfile
import time
import unittest


SCRIPT_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPT_DIR))

import ollama_qwen_smoke as smoke  # noqa: E402


class OllamaQwenSmokeTest(unittest.TestCase):
    def test_select_model_prefers_verified_cached_model(self) -> None:
        self.assertEqual(
            smoke.select_model(None, ["qwen2.5-coder:7b"]), "qwen2.5-coder:7b"
        )
        self.assertEqual(smoke.select_model(None, []), smoke.DEFAULT_MODEL)
        with self.assertRaisesRegex(ValueError, "Qwen"):
            smoke.select_model("llama3.2:1b", [])

    def test_interpreter_command_selects_chat_completions(self) -> None:
        command = smoke.build_interpreter_command(
            Path("interpreter"), Path("workspace"), smoke.DEFAULT_MODEL, "do the task"
        )

        self.assertIn("--chat-completions", command)

    def test_command_runner_drains_output_without_exceeding_bound(self) -> None:
        result = smoke.run_bounded_command(
            [sys.executable, "-c", "print('x' * 1000)"],
            timeout_seconds=10,
            max_bytes=128,
        )

        self.assertEqual(result.returncode, 0)
        self.assertTrue(result.truncated)
        self.assertLessEqual(len(result.output), 128)

    @unittest.skipUnless(
        os.name == "posix", "process-group semantics are POSIX-specific"
    )
    def test_command_runner_times_out_sparse_output_and_cleans_process_group(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as directory:
            descendant_pid_path = Path(directory) / "descendant.pid"
            child = (
                "import pathlib, subprocess, sys, time\n"
                "descendant = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(30)'])\n"
                "pathlib.Path(sys.argv[1]).write_text(str(descendant.pid), encoding='ascii')\n"
                "print('prefix', end='', flush=True)\n"
                "time.sleep(30)\n"
            )
            started = time.monotonic()
            result = smoke.run_bounded_command(
                [sys.executable, "-c", child, str(descendant_pid_path)],
                timeout_seconds=0.2,
                max_bytes=128,
            )
            elapsed = time.monotonic() - started

            self.assertTrue(result.timed_out)
            self.assertLess(elapsed, 2)
            self.assertEqual(result.output, b"prefix")
            self.assertEqual(result.total_bytes, len(result.output))
            self.assertFalse(result.truncated)

            descendant_pid = int(descendant_pid_path.read_text(encoding="ascii"))
            process_deadline = time.monotonic() + 2
            while time.monotonic() < process_deadline:
                try:
                    os.kill(descendant_pid, 0)
                except ProcessLookupError:
                    break
                if sys.platform == "linux":
                    proc_stat = Path(f"/proc/{descendant_pid}/stat")
                    if proc_stat.exists() and proc_stat.read_text().split()[2] == "Z":
                        break
                time.sleep(0.01)
            else:
                self.fail("timed-out command descendant survived process-group cleanup")

            side_effect = smoke.SideEffectResult(False, "result.txt", "file_missing")
            self.assertEqual(
                smoke.classify_attempt(
                    returncode=result.returncode,
                    timed_out=result.timed_out,
                    trace=result.output.decode("utf-8"),
                    side_effect=side_effect,
                ),
                "interpreter_loop",
            )

    @unittest.skipUnless(
        os.name == "posix", "process-group semantics are POSIX-specific"
    )
    def test_command_runner_cleans_descendant_after_leader_exits(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            descendant_pid_path = Path(directory) / "descendant.pid"
            child = (
                "import pathlib, subprocess, sys, time\n"
                "descendant = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(30)'])\n"
                "pathlib.Path(sys.argv[1]).write_text(str(descendant.pid), encoding='ascii')\n"
                "print('prefix', end='', flush=True)\n"
            )
            result = smoke.run_bounded_command(
                [sys.executable, "-c", child, str(descendant_pid_path)],
                timeout_seconds=2,
                max_bytes=128,
            )

            self.assertEqual(result.returncode, 0)
            self.assertFalse(result.timed_out)
            self.assertEqual(result.output, b"prefix")
            descendant_pid = int(descendant_pid_path.read_text(encoding="ascii"))
            process_deadline = time.monotonic() + 2
            while time.monotonic() < process_deadline:
                try:
                    os.kill(descendant_pid, 0)
                except ProcessLookupError:
                    break
                if sys.platform == "linux":
                    proc_stat = Path(f"/proc/{descendant_pid}/stat")
                    if proc_stat.exists() and proc_stat.read_text().split()[2] == "Z":
                        break
                time.sleep(0.01)
            else:
                self.fail(
                    "descendant survived cleanup after its process-group leader exited"
                )

    def test_side_effect_requires_exact_content(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            workspace = Path(directory)
            expected = smoke.verify_side_effect(workspace, "result.txt", "expected\n")
            self.assertEqual(expected.reason, "file_missing")

            (workspace / "result.txt").write_text("wrong\n", encoding="utf-8")
            mismatch = smoke.verify_side_effect(workspace, "result.txt", "expected\n")
            self.assertEqual(mismatch.reason, "content_mismatch")

            (workspace / "result.txt").write_text("expected\n", encoding="utf-8")
            verified = smoke.verify_side_effect(workspace, "result.txt", "expected\n")
            self.assertTrue(verified.verified)

    def test_classification_separates_model_and_interpreter_failures(self) -> None:
        missing = smoke.SideEffectResult(False, "result.txt", "file_missing")
        verified = smoke.SideEffectResult(True, "result.txt", "exact_content_match")
        cases = (
            (
                "model_incompatible_no_tool_call",
                0,
                False,
                '{"type":"message"}',
                missing,
            ),
            ("interpreter_loop", None, True, '{"type":"command_execution"}', missing),
            ("interpreter_crash", -11, False, "", missing),
            ("passed", 0, False, '{"type":"function_call"}', verified),
        )
        for expected, returncode, timed_out, trace, side_effect in cases:
            with self.subTest(expected=expected):
                self.assertEqual(
                    smoke.classify_attempt(
                        returncode=returncode,
                        timed_out=timed_out,
                        trace=trace,
                        side_effect=side_effect,
                    ),
                    expected,
                )


if __name__ == "__main__":
    unittest.main()
