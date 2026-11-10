"""Small fixture checks; no release assets or GitHub credentials are needed."""

import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).with_name("oix-release-provenance.py")
REF = "refs/tags/rust-v9.9.9"
COMMIT = "a" * 40
NAMES = [f"open-interpreter-package-{target}.{suffix}"
         for target in ("aarch64-unknown-linux-musl", "x86_64-unknown-linux-musl")
         for suffix in ("tar.gz", "tar.zst")]


class ReleaseProvenanceTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.dist = self.root / "dist"
        self.dist.mkdir()
        for name in NAMES:
            (self.dist / name).write_bytes(name.encode())
        bin_dir = self.root / "bin"
        bin_dir.mkdir()
        gh = bin_dir / "gh"
        gh.write_text("#!/bin/sh\n"
                      "printf '%s\\n' \"$*\" >> \"$GH_CALLS\"\n"
                      "[ \"${GH_FAIL:-}\" != \"$(basename \"$3\")\" ]\n")
        gh.chmod(0o755)
        self.calls = self.root / "calls"
        self.env = {**os.environ, "PATH": f"{bin_dir}:{os.environ['PATH']}",
                    "GH_CALLS": str(self.calls)}

    def run_script(self, mode):
        return subprocess.run(
            [sys.executable, str(SCRIPT), mode, "--dist", str(self.dist),
             "--repo", "openinterpreter/openinterpreter", "--ref", REF,
             "--commit", COMMIT, "--run-id", "123"],
            env=self.env, capture_output=True, text=True,
        )

    def test_original_archives_and_manifest_verify(self):
        self.assertEqual(self.run_script("prepare").returncode, 0)
        self.assertEqual(self.run_script("verify").returncode, 0)
        calls = self.calls.read_text().splitlines()
        self.assertEqual(len(calls), 9)
        self.assertTrue(all("--source-ref " + REF in c and
                            "--source-digest " + COMMIT in c and
                            "--signer-workflow openinterpreter/openinterpreter/.github/workflows/rust-release.yml" in c
                            for c in calls))
        self.assertIn("open-interpreter-package_SHA256SUMS", calls[-1])

    def test_missing_or_changed_asset_fails_closed(self):
        self.assertEqual(self.run_script("prepare").returncode, 0)
        (self.dist / NAMES[0]).write_bytes(b"substituted")
        self.assertNotEqual(self.run_script("verify").returncode, 0)
        (self.dist / NAMES[0]).unlink()
        self.assertNotEqual(self.run_script("verify").returncode, 0)

    def test_missing_or_tampered_original_manifest_fails_closed(self):
        self.assertNotEqual(self.run_script("verify").returncode, 0)
        self.assertEqual(self.run_script("prepare").returncode, 0)
        (self.dist / "open-interpreter-package_SHA256SUMS").write_bytes(b"fake\n")
        self.assertNotEqual(self.run_script("verify").returncode, 0)

    def test_failed_attestation_fails_closed(self):
        self.env["GH_FAIL"] = NAMES[0]
        self.assertNotEqual(self.run_script("prepare").returncode, 0)
        self.assertFalse((self.dist / "open-interpreter-package_SHA256SUMS").exists())


if __name__ == "__main__":
    unittest.main()
