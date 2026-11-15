"""Exercise the release tag gate without dispatching a publishing workflow."""

import os
from pathlib import Path
import subprocess
import tempfile
import textwrap
import unittest


WORKFLOW = Path(__file__).resolve().parents[1] / "workflows" / "rust-release.yml"


class ReleaseTagValidationTest(unittest.TestCase):
    def test_tag_version_is_defined_before_comparison(self):
        workflow = WORKFLOW.read_text(encoding="utf-8")
        gate = workflow.split(
            "      - name: Validate tag matches Cargo.toml version\n", 1
        )[1]
        gate = gate.split('          echo "::endgroup::"', 1)[0]
        script = textwrap.dedent(gate.split("        run: |\n", 1)[1])

        for ref, version, accepted in (
            ("rust-v0.0.62", "0.0.62", True),
            ("rust-v0.0.62-beta.1", "0.0.62-beta.1", True),
            ("rust-v0.0.63", "0.0.62", False),
            ("not-a-tag", "0.0.62", False),
        ):
            with self.subTest(ref=ref, version=version):
                with tempfile.TemporaryDirectory() as root:
                    cargo = Path(root) / "codex-rs" / "Cargo.toml"
                    cargo.parent.mkdir()
                    cargo.write_text(f'version = "{version}"\n', encoding="utf-8")
                    result = subprocess.run(
                        ["bash", "-c", script],
                        cwd=root,
                        env={
                            **os.environ,
                            "GITHUB_REF_TYPE": "tag",
                            "GITHUB_REF_NAME": ref,
                        },
                        capture_output=True,
                        text=True,
                        check=False,
                    )
                self.assertEqual(result.returncode == 0, accepted, result.stderr)


if __name__ == "__main__":
    unittest.main()
