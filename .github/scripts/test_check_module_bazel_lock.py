"""Fail-closed, bounded machine diagnostics for the Bazel lock verification step."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


CHECK = Path(__file__).resolve().parents[2] / "scripts/check-module-bazel-lock.sh"


class CheckModuleBazelLockTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        root = Path(self.temp.name)
        script = root / "scripts/check-module-bazel-lock.sh"
        script.parent.mkdir()
        shutil.copy2(CHECK, script)
        self.script = script
        runner = root / ".github/scripts/run_bazel_with_buildbuddy.py"
        runner.parent.mkdir(parents=True)
        runner.write_text(
            "#!/usr/bin/env bash\n"
            '[[ "$*" == "mod deps --lockfile_mode=error" ]] || exit 90\n'
            'printf "%s\\n" "$FAKE_BAZEL_OUTPUT" >&2\n'
            'exit "$FAKE_BAZEL_EXIT"\n',
            encoding="utf-8",
        )
        runner.chmod(0o755)

    def run_check(self, output, status=17):
        env = os.environ.copy()
        env.update(FAKE_BAZEL_OUTPUT=output, FAKE_BAZEL_EXIT=str(status))
        return subprocess.run(
            [str(self.script)], env=env, capture_output=True, text=True, check=False
        )

    def test_patch_failure_not_stale_lock(self):
        proc = self.run_check(
            "INFO: lockfile may need update\n"
            "ERROR: failed to apply patch to external repository\n"
            "ERROR: repository failed to fetch"
        )
        self.assertEqual(proc.returncode, 17)
        self.assertEqual(proc.stderr, "")
        record = json.loads(proc.stdout)
        self.assertEqual(record["class"], "patch_failure")
        self.assertEqual(record["bazel_exit_code"], 17)
        self.assertEqual(record["check"], "module_bazel_lock")

    def test_stale_lock_not_ordinary_guidance(self):
        proc = self.run_check("ERROR: MODULE.bazel.lock is no longer up-to-date")
        self.assertEqual(proc.returncode, 17)
        self.assertEqual(json.loads(proc.stdout)["class"], "stale_lock")

    def test_exact_patch_content_mismatch_classified(self):
        proc = self.run_check(
            "ERROR: CONTENT_DOES_NOT_MATCH_TARGET in pinned repository"
        )
        self.assertEqual(proc.returncode, 17)
        self.assertEqual(json.loads(proc.stdout)["class"], "patch_failure")

    def test_success_retains_exact_check(self):
        proc = self.run_check("INFO: dependencies resolved", status=0)
        self.assertEqual(proc.returncode, 0)
        self.assertEqual(proc.stdout.strip(), "MODULE.bazel.lock verified")

    def test_bounded_and_redacted(self):
        output = "\n".join(
            ["ERROR: downloading https://example.invalid/resource?token=secret"]
            + [f"ERROR: failure {i} authorization:abc" for i in range(20)]
        )
        proc = self.run_check(output)
        record = json.loads(proc.stdout)
        self.assertEqual(record["error_lines_total"], 21)
        self.assertEqual(len(record["errors"]), 4)
        self.assertTrue(record["truncated"])
        self.assertNotIn("example.invalid", proc.stdout)
        self.assertNotIn("authorization:abc", proc.stdout)
        self.assertNotIn("token=secret", proc.stdout)


if __name__ == "__main__":
    unittest.main()
