"""Guard signed and unsigned Windows voice packaging in the release workflow."""

import re
import unittest
from pathlib import Path


WORKFLOW = Path(__file__).resolve().parents[1] / "workflows/rust-release-windows.yml"


def step(source: str, name: str) -> str:
    match = re.search(
        rf"(?ms)^      - name: {re.escape(name)}\n(.*?)(?=^      - (?:name:|uses:)|\Z)",
        source,
    )
    if match is None:
        raise AssertionError(f"missing step: {name}")
    return match.group(1)


class WindowsReleaseArtifactsTest(unittest.TestCase):
    def test_sign_only_when_signed_but_seal_both_modes(self) -> None:
        workflow = WORKFLOW.read_text(encoding="utf-8")
        signing = step(workflow, "Sign Windows voice helper and native DLLs")
        self.assertIn("if: ${{ inputs.signing_mode == 'signed' }}", signing)
        sealing = step(workflow, "Seal and verify mode-selected voice output")
        self.assertIn("$signingMode -notin @('signed', 'unsigned')", sealing)
        self.assertIn("if ($signingMode -eq 'signed') {", sealing)
        self.assertIn("Get-AuthenticodeSignature $file", sealing)
        self.assertIn("release_runtime.py') seal", sealing)
        self.assertIn('"VOICE_RELEASE_DIR=$signed"', sealing)

    def test_packaged_unsigned_voice_is_verified_without_a_signature(self) -> None:
        workflow = WORKFLOW.read_text(encoding="utf-8")
        packaging = step(workflow, "Build Codex package archives")
        self.assertIn('voice_args=(--voice-release-dir "$VOICE_RELEASE_DIR")', packaging)
        verification = step(workflow, "Verify packaged Windows voice closure")
        self.assertIn('if ("${{ inputs.signing_mode }}" -eq \'signed\') {', verification)
        self.assertIn("Get-AuthenticodeSignature $helper", verification)
        self.assertIn("public_release=True", verification)


if __name__ == "__main__":
    unittest.main()
