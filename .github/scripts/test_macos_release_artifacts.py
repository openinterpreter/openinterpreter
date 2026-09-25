"""Guard both signing-mode artifact paths in the macOS release workflow."""

import re
import unittest
from pathlib import Path


WORKFLOW = Path(__file__).resolve().parents[1] / "workflows/rust-release.yml"


def job(name: str) -> str:
    source = WORKFLOW.read_text(encoding="utf-8")
    match = re.search(rf"(?ms)^  {re.escape(name)}:\n(.*?)(?=^  [\w-]+:\n|\Z)", source)
    if match is None:
        raise AssertionError(f"missing job: {name}")
    return match.group(1)


def step(source: str, name: str) -> str:
    match = re.search(
        rf"(?ms)^      - name: {re.escape(name)}\n(.*?)(?=^      - (?:name:|uses:)|\Z)",
        source,
    )
    if match is None:
        raise AssertionError(f"missing step: {name}")
    return match.group(1)


class MacosReleaseArtifactsTest(unittest.TestCase):
    def test_both_modes_select_existing_helper_artifact_for_every_bundle(self) -> None:
        packaging = job("package-macos")
        download = step(packaging, "Download mode-selected macOS helpers")
        self.assertIn(
            "${{ matrix.target }}-${{ needs.resolve_macos_signing_mode.outputs.signing_mode }}-resources",
            download,
        )
        self.assertIn("path: codex-rs/macos-resources/${{ matrix.target }}", download)
        self.assertNotIn("if: ${{ matrix.bundle", download)
        self.assertIn(
            "name: ${{ matrix.target }}-signed-resources", job("sign-macos-binaries")
        )
        self.assertIn(
            "name: ${{ matrix.target }}-unsigned-resources",
            job("prepare-unsigned-macos"),
        )
        self.assertNotIn('"signed-resources/', packaging)
        for name in (
            "Build Codex package archive",
            "Build Open Interpreter package archive",
            "Build Python runtime wheel",
        ):
            script = step(packaging, name)
            self.assertIn('--rg-bin "macos-resources/', script)
            self.assertIn('--zsh-bin "macos-resources/', script)

    def test_primary_voice_uses_selected_mode_and_verified_neutral_path(self) -> None:
        packaging = job("package-macos")
        self.assertIn("- build-macos-voice", packaging)
        self.assertIn("needs.build-macos-voice.result == 'success'", packaging)
        download = step(packaging, "Download mode-selected voice runtime")
        self.assertIn("if: ${{ matrix.bundle == 'primary' }}", download)
        self.assertIn(
            "name: voice-${{ matrix.target }}-${{ needs.resolve_macos_signing_mode.outputs.signing_mode }}",
            download,
        )
        self.assertIn("path: ${{ runner.temp }}/release-voice", download)
        verify = step(packaging, "Prepare and verify mode-selected voice runtime")
        for item in (
            "signed-voice-${TARGET}.tar.gz",
            "voice-unsigned-${TARGET}.tar.gz",
            'release_runtime.py" stage',
            'release_runtime.py" seal',
            "runtime_files(Path(sys.argv[1]).resolve(strict=True), sys.argv[2], public_release=True)",
            'if [[ "$MACOS_SIGNING_MODE" == "signed" ]]',
        ):
            self.assertIn(item, verify)
        archive = step(packaging, "Build Codex package archive")
        self.assertIn(
            '--voice-release-dir "${RUNNER_TEMP}/release-voice/${TARGET}"', archive
        )
        self.assertIn(
            "name: voice-${{ matrix.target }}-signed", job("sign-macos-binaries")
        )
        self.assertIn(
            "name: voice-${{ matrix.target }}-unsigned", job("build-macos-voice")
        )

    def test_final_verifier_checks_voice_in_both_modes_and_signatures_only_when_signed(
        self,
    ) -> None:
        verify = step(job("finalize-macos"), "Verify mode-selected macOS artifacts")
        self.assertIn(
            "runtime_files(voice.resolve(strict=True), str(target), public_release=True)",
            verify,
        )
        self.assertIn(
            'if [[ "$MACOS_SIGNING_MODE" == "signed" ]]; then\n              verify_signed_binary "$helper"',
            verify,
        )
        self.assertIn(
            'else\n              lipo "$helper" -verify_arch "$expected_arch"', verify
        )


if __name__ == "__main__":
    unittest.main()
