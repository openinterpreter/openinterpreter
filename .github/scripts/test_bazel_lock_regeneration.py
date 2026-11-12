"""Provenance and fail-closed fixtures for review-only Bazel lock candidates."""

import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("verify_bazel_lock_regeneration.py")


class LockCandidateTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.repo = Path(self.temp.name)
        self.git("init", "-q")
        (self.repo / "MODULE.bazel.lock").write_bytes(b"old lock\n")
        (self.repo / "MODULE.bazel").write_text("module(name = 'test')\n", encoding="utf-8")
        self.git("add", "MODULE.bazel.lock", "MODULE.bazel")
        self.git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "-qm", "fixture")
        self.head = self.git("rev-parse", "HEAD").stdout.strip()

    def git(self, *args):
        return subprocess.run(
            ["git", *args], cwd=self.repo, capture_output=True, text=True, check=True
        )

    def generate(self, source=None, output="review"):
        return subprocess.run(
            [sys.executable, str(SCRIPT), "--source-sha", source or self.head,
             "--run-id", "123456", "--run-attempt", "2", "--output-dir", output],
            cwd=self.repo, capture_output=True, text=True, check=False,
        )

    def test_bound_and_exact_candidate(self):
        (self.repo / "MODULE.bazel.lock").write_bytes(b"new lock\n")
        proc = self.generate()
        self.assertEqual(proc.returncode, 0, proc.stderr)
        manifest = json.loads((self.repo / "review/provenance.json").read_text(encoding="utf-8"))
        self.assertEqual(manifest, json.loads(proc.stdout))
        self.assertEqual(manifest["source_sha"], self.head)
        self.assertEqual(manifest["run_attempt"], 2)
        self.assertEqual(manifest["candidate_lock_sha256"], hashlib.sha256(b"new lock\n").hexdigest())
        self.assertEqual((self.repo / "review/MODULE.bazel.lock").read_bytes(), b"new lock\n")

    def test_reject_wrong_source(self):
        (self.repo / "MODULE.bazel.lock").write_bytes(b"new lock\n")
        self.assertNotEqual(self.generate(source="a" * 40).returncode, 0)
        self.assertFalse((self.repo / "review").exists())

    def test_reject_unchanged_or_mixed_tree(self):
        self.assertNotEqual(self.generate().returncode, 0)
        (self.repo / "MODULE.bazel.lock").write_bytes(b"new lock\n")
        (self.repo / "MODULE.bazel").write_text("module(name='different')\n", encoding="utf-8")
        self.assertNotEqual(self.generate().returncode, 0)
        self.assertFalse((self.repo / "review").exists())

    def test_refuse_replace_existing_review(self):
        (self.repo / "MODULE.bazel.lock").write_bytes(b"new lock\n")
        (self.repo / "review").mkdir()
        self.assertNotEqual(self.generate().returncode, 0)


if __name__ == "__main__":
    unittest.main()
