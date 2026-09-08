from __future__ import annotations

import os
from pathlib import Path
import subprocess
import tempfile
import textwrap
import unittest


REPO_ROOT = Path(__file__).resolve().parents[1]
BUILD_SCRIPT = REPO_ROOT / "scripts" / "build-interpreter-release.sh"


class BuildInterpreterReleaseTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temp_dir = tempfile.TemporaryDirectory()
        self.root = Path(self.temp_dir.name)
        self.fake_bin = self.root / "fake-bin"
        self.fake_bin.mkdir()
        self.invocation = self.root / "invocation.txt"
        fake_python = self.fake_bin / "python3"
        fake_python.write_text(
            textwrap.dedent(
                """\
                #!/usr/bin/env bash
                set -euo pipefail
                if [[ "${1:-}" == "-c" ]]; then
                  exit 0
                fi
                printf 'jobs=%s\\n' "${CARGO_BUILD_JOBS:-}" > "$TEST_INVOCATION"
                printf 'repo_root=%s\\n' "${CODEX_REPO_ROOT:-}" >> "$TEST_INVOCATION"
                printf '%s\\n' "$@" >> "$TEST_INVOCATION"
                package_dir=""
                while [[ $# -gt 0 ]]; do
                  if [[ "$1" == "--package-dir" ]]; then
                    package_dir="$2"
                    break
                  fi
                  shift
                done
                mkdir -p "$package_dir/bin"
                cat > "$package_dir/bin/interpreter" <<'EOF'
                #!/usr/bin/env sh
                echo "Open Interpreter test"
                EOF
                chmod +x "$package_dir/bin/interpreter"
                """
            ),
            encoding="utf-8",
        )
        fake_python.chmod(0o755)

    def tearDown(self) -> None:
        self.temp_dir.cleanup()

    def run_script(self, *args: str) -> subprocess.CompletedProcess[str]:
        env = os.environ.copy()
        env["PATH"] = f"{self.fake_bin}{os.pathsep}{env['PATH']}"
        env["TEST_INVOCATION"] = str(self.invocation)
        return subprocess.run(
            [str(BUILD_SCRIPT), *args],
            cwd=self.root,
            env=env,
            text=True,
            capture_output=True,
            check=False,
        )

    def test_help_describes_release_safe_options(self) -> None:
        result = self.run_script("--help")

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("--jobs", result.stdout)
        self.assertNotIn("--cargo-profile", result.stdout)

    def test_equals_options_and_explicit_jobs_build_release_layout(self) -> None:
        result = self.run_script(
            "--target=aarch64-apple-darwin",
            "--install-dir=visible-bin",
            "--home=interpreter-home",
            "--jobs=3",
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        invocation = self.invocation.read_text(encoding="utf-8")
        self.assertIn("jobs=3\n", invocation)
        self.assertIn("--target\naarch64-apple-darwin\n", invocation)
        self.assertIn("--cargo-profile\nrelease\n", invocation)
        self.assertTrue((self.root / "visible-bin" / "interpreter").is_symlink())
        self.assertTrue((self.root / "visible-bin" / "i").is_symlink())

    def test_builder_receives_script_repository_root_from_other_cwd(self) -> None:
        result = self.run_script(
            "--install-dir",
            "visible-bin",
            "--home",
            "interpreter-home",
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(
            f"repo_root={REPO_ROOT}\n",
            self.invocation.read_text(encoding="utf-8"),
        )

    def test_default_jobs_remains_one(self) -> None:
        result = self.run_script(
            "--install-dir",
            "visible-bin",
            "--home",
            "interpreter-home",
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("jobs=1\n", self.invocation.read_text(encoding="utf-8"))

    def test_invalid_jobs_are_rejected_before_build(self) -> None:
        result = self.run_script("--jobs", "0")

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("positive integer", result.stderr)
        self.assertFalse(self.invocation.exists())

    def test_existing_directory_is_not_replaced_by_shim(self) -> None:
        blocked_shim = self.root / "visible-bin" / "interpreter"
        blocked_shim.mkdir(parents=True)

        result = self.run_script(
            "--install-dir",
            "visible-bin",
            "--home",
            "interpreter-home",
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Refusing to replace non-symlink directory", result.stderr)
        self.assertTrue(blocked_shim.is_dir())


if __name__ == "__main__":
    unittest.main()
