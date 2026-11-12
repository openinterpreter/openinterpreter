#!/usr/bin/env python3

from pathlib import Path
import os
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from codex_package import cargo as cargo_module
from codex_package.cargo import build_source_binaries
from codex_package.cargo import source_binaries_for_target
from codex_package.targets import PACKAGE_VARIANTS
from codex_package.targets import TARGET_SPECS


class SourceBinariesForTargetTest(unittest.TestCase):
    def test_macos_package_with_prebuilt_entrypoint_builds_nothing(self) -> None:
        self.assertEqual(
            source_binaries_for_target(
                TARGET_SPECS["aarch64-apple-darwin"],
                PACKAGE_VARIANTS["codex"],
                build_entrypoint=False,
                build_managed_codex=False,
                build_code_mode_host=False,
                build_exec_server=False,
                build_bwrap=False,
                build_codex_command_runner=False,
                build_codex_windows_sandbox_setup=False,
            ),
            [],
        )

    def test_linux_package_with_prebuilt_entrypoint_and_bwrap_builds_nothing(
        self,
    ) -> None:
        self.assertEqual(
            source_binaries_for_target(
                TARGET_SPECS["x86_64-unknown-linux-musl"],
                PACKAGE_VARIANTS["codex"],
                build_entrypoint=False,
                build_managed_codex=False,
                build_code_mode_host=False,
                build_exec_server=False,
                build_bwrap=False,
                build_codex_command_runner=False,
                build_codex_windows_sandbox_setup=False,
            ),
            [],
        )

    def test_windows_package_with_prebuilt_entrypoint_and_helpers_builds_nothing(
        self,
    ) -> None:
        self.assertEqual(
            source_binaries_for_target(
                TARGET_SPECS["x86_64-pc-windows-msvc"],
                PACKAGE_VARIANTS["codex"],
                build_entrypoint=False,
                build_managed_codex=False,
                build_code_mode_host=False,
                build_exec_server=False,
                build_bwrap=False,
                build_codex_command_runner=False,
                build_codex_windows_sandbox_setup=False,
            ),
            [],
        )

    def test_missing_windows_helpers_are_built(self) -> None:
        self.assertEqual(
            source_binaries_for_target(
                TARGET_SPECS["x86_64-pc-windows-msvc"],
                PACKAGE_VARIANTS["codex"],
                build_entrypoint=False,
                build_managed_codex=False,
                build_code_mode_host=False,
                build_exec_server=False,
                build_bwrap=False,
                build_codex_command_runner=True,
                build_codex_windows_sandbox_setup=True,
            ),
            ["codex-command-runner", "codex-windows-sandbox-setup"],
        )

    def test_missing_code_mode_host_is_built_for_app_server(self) -> None:
        self.assertEqual(
            source_binaries_for_target(
                TARGET_SPECS["aarch64-apple-darwin"],
                PACKAGE_VARIANTS["codex-app-server"],
                build_entrypoint=False,
                build_managed_codex=False,
                build_code_mode_host=True,
                build_exec_server=True,
                build_bwrap=False,
                build_codex_command_runner=False,
                build_codex_windows_sandbox_setup=False,
            ),
            ["codex-code-mode-host", "codex-exec-server"],
        )

    def test_build_uses_prebuilt_windows_helpers_without_running_cargo(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            entrypoint = touch_file(root / "codex.exe")
            code_mode_host = touch_file(root / "codex-code-mode-host.exe")
            command_runner = touch_file(root / "codex-command-runner.exe")
            sandbox_setup = touch_file(root / "codex-windows-sandbox-setup.exe")

            outputs = build_source_binaries(
                TARGET_SPECS["x86_64-pc-windows-msvc"],
                PACKAGE_VARIANTS["codex"],
                cargo=str(root / "cargo-that-should-not-run"),
                profile="release",
                entrypoint_bin=entrypoint,
                managed_codex_bin=None,
                code_mode_host_bin=code_mode_host,
                exec_server_bin=code_mode_host,
                bwrap_bin=None,
                codex_command_runner_bin=command_runner,
                codex_windows_sandbox_setup_bin=sandbox_setup,
            )

        self.assertEqual(outputs.entrypoint_bin, entrypoint)
        self.assertEqual(outputs.code_mode_host_bin, code_mode_host)
        self.assertEqual(outputs.codex_command_runner_bin, command_runner)
        self.assertEqual(outputs.codex_windows_sandbox_setup_bin, sandbox_setup)

    def test_open_interpreter_package_uses_codex_multitool_entrypoint(self) -> None:
        self.assertFalse(PACKAGE_VARIANTS["open-interpreter"].managed_codex_required)
        self.assertEqual(
            source_binaries_for_target(
                TARGET_SPECS["aarch64-apple-darwin"],
                PACKAGE_VARIANTS["open-interpreter"],
                build_entrypoint=True,
                build_managed_codex=False,
                build_code_mode_host=False,
                build_exec_server=False,
                build_bwrap=False,
                build_codex_command_runner=False,
                build_codex_windows_sandbox_setup=False,
            ),
            ["codex"],
        )

    def test_open_interpreter_source_build_includes_code_mode_host(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            cargo = write_fake_cargo(root / "cargo")
            target_dir = root / "target"
            original_target_dir = os.environ.get("CARGO_TARGET_DIR")
            original_resolver = cargo_module.resolve_codex_v8_cargo_env
            os.environ["CARGO_TARGET_DIR"] = str(target_dir)
            cargo_module.resolve_codex_v8_cargo_env = empty_v8_env
            try:
                outputs = build_source_binaries(
                    TARGET_SPECS["aarch64-apple-darwin"],
                    PACKAGE_VARIANTS["open-interpreter"],
                    cargo=str(cargo),
                    profile="release",
                    entrypoint_bin=None,
                    managed_codex_bin=None,
                    code_mode_host_bin=None,
                    exec_server_bin=None,
                    bwrap_bin=None,
                    codex_command_runner_bin=None,
                    codex_windows_sandbox_setup_bin=None,
                )
            finally:
                cargo_module.resolve_codex_v8_cargo_env = original_resolver
                if original_target_dir is None:
                    os.environ.pop("CARGO_TARGET_DIR", None)
                else:
                    os.environ["CARGO_TARGET_DIR"] = original_target_dir

        self.assertEqual(
            outputs.entrypoint_bin,
            target_dir / "aarch64-apple-darwin" / "release" / "codex",
        )
        self.assertIsNone(outputs.managed_codex_bin)
        self.assertEqual(
            outputs.code_mode_host_bin,
            target_dir / "aarch64-apple-darwin" / "release" / "codex-code-mode-host",
        )


def touch_file(path: Path) -> Path:
    path.write_text("", encoding="utf-8")
    return path.resolve()


def write_fake_cargo(path: Path) -> Path:
    path.write_text(
        "\n".join(
            [
                "#!/bin/sh",
                "set -eu",
                'target=""',
                'profile=""',
                'bins=""',
                'while [ "$#" -gt 0 ]; do',
                '  case "$1" in',
                "    --target)",
                '      target="$2"',
                "      shift 2",
                "      ;;",
                "    --profile)",
                '      profile="$2"',
                "      shift 2",
                "      ;;",
                "    --bin)",
                '      bins="${bins} $2"',
                "      shift 2",
                "      ;;",
                "    *)",
                "      shift",
                "      ;;",
                "  esac",
                "done",
                'out="${CARGO_TARGET_DIR}/${target}/${profile}"',
                'mkdir -p "$out"',
                'for bin in $bins; do touch "$out/$bin"; done',
                "",
            ]
        ),
        encoding="utf-8",
    )
    path.chmod(0o755)
    return path.resolve()


def empty_v8_env(*_args: object, **_kwargs: object) -> dict[str, str]:
    return {}


if __name__ == "__main__":
    unittest.main()
