#!/usr/bin/env python3

from pathlib import Path
import hashlib
import json
import struct
import sys
import tempfile
import unittest
from dataclasses import replace

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from build_winget_package import prepare_winget_package
from codex_package.layout import build_package_dir
from codex_package.layout import prepare_package_dir
from codex_package.layout import validate_package_dir
from codex_package.layout import verify_llama_server_input
from codex_package.targets import PACKAGE_VARIANTS
from codex_package.targets import PackageInputs
from codex_package.targets import TARGET_SPECS


class PackageLayoutTest(unittest.TestCase):
    def test_llama_server_binary_must_match_target_cpu_and_format(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "llama-server"
            cases = (
                ("x86_64-unknown-linux-musl", fake_elf(62)),
                ("aarch64-unknown-linux-gnu", fake_elf(183)),
                ("x86_64-apple-darwin", fake_macho(0x01000007)),
                ("aarch64-apple-darwin", fake_macho(0x0100000C)),
                ("x86_64-pc-windows-msvc", fake_pe(0x8664)),
                ("aarch64-pc-windows-msvc", fake_pe(0xAA64)),
            )
            for target, contents in cases:
                with self.subTest(target=target):
                    path.write_bytes(contents)
                    digest = hashlib.sha256(contents).hexdigest()
                    verify_llama_server_input(path, digest, TARGET_SPECS[target])
                    with self.assertRaisesRegex(RuntimeError, "architecture does not match"):
                        other = (
                            target.replace("x86_64-", "aarch64-")
                            if target.startswith("x86_64-")
                            else target.replace("aarch64-", "x86_64-")
                        )
                        verify_llama_server_input(path, digest, TARGET_SPECS[other])
                    wrong_os = (
                        "x86_64-apple-darwin"
                        if "linux" in target
                        else "x86_64-unknown-linux-musl"
                    )
                    with self.assertRaisesRegex(RuntimeError, "llama-server is not|requires a thin"):
                        verify_llama_server_input(path, digest, TARGET_SPECS[wrong_os])

    def test_open_interpreter_package_contains_interpreter_alias_and_code_mode_host(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            inputs = PackageInputs(
                entrypoint_bin=touch_executable(root / "interpreter"),
                managed_codex_bin=None,
                code_mode_host_bin=touch_executable(root / "codex-code-mode-host"),
                rg_bin=touch_executable(root / "rg"),
                zsh_bin=None,
                bwrap_bin=touch_executable(root / "bwrap"),
                codex_command_runner_bin=None,
                codex_windows_sandbox_setup_bin=None,
                llama_server_bin=touch_executable(root / "llama-server"),
                llama_server_sha256=hashlib.sha256(fake_elf(62)).hexdigest(),
            )
            inputs.llama_server_bin.write_bytes(fake_elf(62))
            package_dir = root / "package"

            prepare_package_dir(package_dir, force=False)
            build_package_dir(
                package_dir,
                "1.2.3",
                PACKAGE_VARIANTS["open-interpreter"],
                TARGET_SPECS["x86_64-unknown-linux-musl"],
                inputs,
            )
            validate_package_dir(
                package_dir,
                PACKAGE_VARIANTS["open-interpreter"],
                TARGET_SPECS["x86_64-unknown-linux-musl"],
                include_zsh=False,
            )

            self.assertEqual(
                (package_dir / "bin" / "interpreter").read_text(),
                "interpreter",
            )
            self.assertEqual((package_dir / "bin" / "i").read_text(), "interpreter")
            self.assertEqual(
                (package_dir / "codex-resources" / "llama.cpp" / "llama-server").read_bytes(),
                fake_elf(62),
            )
            self.assertEqual(
                (package_dir / "bin" / "codex-code-mode-host").read_text(),
                "codex-code-mode-host",
            )
            self.assertFalse((package_dir / "bin" / "codex").exists())
            metadata = (package_dir / "codex-package.json").read_text(encoding="utf-8")
            self.assertIn('"entrypoint": "bin/interpreter"', metadata)
            self.assertIn(
                '"llamaServer": "codex-resources/llama.cpp/llama-server"', metadata
            )
            self.assertIn('"llamaServerSha256": ', metadata)
            packaged_llama = package_dir / "codex-resources" / "llama.cpp" / "llama-server"
            packaged_llama.write_bytes(b"tampered executable")
            with self.assertRaisesRegex(RuntimeError, "does not match the pinned digest"):
                validate_package_dir(
                    package_dir,
                    PACKAGE_VARIANTS["open-interpreter"],
                    TARGET_SPECS["x86_64-unknown-linux-musl"],
                    include_zsh=False,
                )
            self.assertNotIn('"managedCodex"', metadata)

            missing_llama_server = replace(inputs, llama_server_bin=None)
            missing_package = root / "missing-llama-package"
            prepare_package_dir(missing_package, force=False)
            with self.assertRaisesRegex(RuntimeError, "require a llama-server binary"):
                build_package_dir(
                    missing_package,
                    "1.2.3",
                    PACKAGE_VARIANTS["open-interpreter"],
                    TARGET_SPECS["x86_64-unknown-linux-musl"],
                    missing_llama_server,
                )

            wrong_digest = replace(inputs, llama_server_sha256="0" * 64)
            with self.assertRaisesRegex(RuntimeError, "does not match the pinned digest"):
                build_package_dir(
                    root / "wrong-digest-package",
                    "1.2.3",
                    PACKAGE_VARIANTS["open-interpreter"],
                    TARGET_SPECS["x86_64-unknown-linux-musl"],
                    wrong_digest,
                )
            self.assertFalse((root / "wrong-digest-package").exists())

    def test_winget_preserves_signed_files_and_voice_hashes(self) -> None:
        for target in ("x86_64-pc-windows-msvc", "aarch64-pc-windows-msvc"):
            with self.subTest(target=target), tempfile.TemporaryDirectory() as temp:
                package = Path(temp)
                files = {
                    "bin/codex.exe": b"signed CLI",
                    "bin/codex-code-mode-host.exe": b"signed code mode host",
                    "codex-resources/codex-command-runner.exe": b"signed runner",
                    "codex-resources/codex-windows-sandbox-setup.exe": b"signed setup",
                    "codex-resources/voice/bin/codex-voice-host.exe": b"signed voice host",
                    "codex-resources/voice/bin/gstreamer-1.0-0.dll": b"signed audio DLL",
                    "codex-resources/voice/NOTICE.md": b"license notices",
                    "codex-path/rg.exe": b"ripgrep",
                }
                for name, contents in files.items():
                    path = package / name
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_bytes(contents)
                metadata = {
                    "layoutVersion": 1,
                    "target": target,
                    "entrypoint": "bin/codex.exe",
                }
                (package / "codex-package.json").write_text(json.dumps(metadata))
                manifest = {
                    "schemaVersion": 1,
                    "sha256": {
                        name: hashlib.sha256(contents).hexdigest()
                        for name, contents in files.items()
                        if name == "bin/codex.exe"
                        or name.startswith("codex-resources/voice/")
                    },
                }
                manifest_path = package / "codex-resources/voice/manifest.json"
                manifest_path.write_text(json.dumps(manifest))
                prepare_winget_package(package)
                entrypoint = f"codex-{target}.exe"
                files[entrypoint] = files.pop("bin/codex.exe")
                files["codex-code-mode-host.exe"] = files.pop(
                    "bin/codex-code-mode-host.exe"
                )
                for helper in (
                    "codex-command-runner.exe",
                    "codex-windows-sandbox-setup.exe",
                ):
                    files[helper] = files[f"codex-resources/{helper}"]
                actual = {
                    str(path.relative_to(package)).replace("\\", "/"): path.read_bytes()
                    for path in package.rglob("*")
                    if path.is_file()
                }
                actual_metadata = json.loads(actual.pop("codex-package.json"))
                actual_manifest = json.loads(
                    actual.pop("codex-resources/voice/manifest.json")
                )
                self.assertEqual(actual, files)
                metadata["entrypoint"] = entrypoint
                self.assertEqual(actual_metadata, metadata)
                manifest["sha256"][entrypoint] = manifest["sha256"].pop("bin/codex.exe")
                self.assertEqual(actual_manifest, manifest)
                for name, digest in actual_manifest["sha256"].items():
                    self.assertEqual(hashlib.sha256(actual[name]).hexdigest(), digest)

    def test_macos_package_preserves_prebuilt_resource_binaries(self) -> None:
        for variant_name in ("codex", "codex-app-server"):
            for target in ("aarch64-apple-darwin", "x86_64-apple-darwin"):
                with self.subTest(variant=variant_name, target=target):
                    with tempfile.TemporaryDirectory() as temp_dir:
                        root = Path(temp_dir)
                        package_dir = root / "package"
                        package_dir.mkdir()
                        rg_bin = touch_executable(root / "signed-rg")
                        zsh_bin = touch_executable(root / "signed-zsh")
                        rg_bin.write_bytes(b"signed ripgrep binary")
                        zsh_bin.write_bytes(b"signed zsh binary")
                        variant = PACKAGE_VARIANTS[variant_name]
                        spec = TARGET_SPECS[target]
                        inputs = PackageInputs(
                            entrypoint_bin=touch_executable(
                                root / variant.executable_stem
                            ),
                            managed_codex_bin=None,
                            code_mode_host_bin=touch_executable(
                                root / "codex-code-mode-host"
                            ),
                            rg_bin=rg_bin,
                            zsh_bin=zsh_bin,
                            bwrap_bin=None,
                            codex_command_runner_bin=None,
                            codex_windows_sandbox_setup_bin=None,
                        )
                        build_package_dir(package_dir, "1.2.3", variant, spec, inputs)
                        validate_package_dir(
                            package_dir, variant, spec, include_zsh=True
                        )

                        self.assertEqual(
                            {
                                "rg": (package_dir / "codex-path" / "rg").read_bytes(),
                                "zsh": (
                                    package_dir
                                    / "codex-resources"
                                    / "zsh"
                                    / "bin"
                                    / "zsh"
                                ).read_bytes(),
                            },
                            {
                                "rg": b"signed ripgrep binary",
                                "zsh": b"signed zsh binary",
                            },
                        )

    def test_app_server_package_places_code_mode_host_beside_entrypoint(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            package_dir = root / "package"
            package_dir.mkdir()
            inputs = PackageInputs(
                entrypoint_bin=touch_executable(root / "codex-app-server"),
                managed_codex_bin=None,
                code_mode_host_bin=touch_executable(root / "codex-code-mode-host"),
                rg_bin=touch_executable(root / "rg"),
                zsh_bin=None,
                bwrap_bin=touch_executable(root / "bwrap"),
                codex_command_runner_bin=None,
                codex_windows_sandbox_setup_bin=None,
            )

            build_package_dir(
                package_dir,
                "1.2.3",
                PACKAGE_VARIANTS["codex-app-server"],
                TARGET_SPECS["x86_64-unknown-linux-musl"],
                inputs,
            )
            validate_package_dir(
                package_dir,
                PACKAGE_VARIANTS["codex-app-server"],
                TARGET_SPECS["x86_64-unknown-linux-musl"],
                include_zsh=False,
            )

            self.assertTrue((package_dir / "bin" / "codex-code-mode-host").is_file())


def touch_executable(path: Path) -> Path:
    path.write_text(path.name, encoding="utf-8")
    path.chmod(0o755)
    return path.resolve()


def fake_elf(machine: int) -> bytes:
    header = bytearray(64)
    header[:6] = b"\x7fELF\x02\x01"
    struct.pack_into("<H", header, 18, machine)
    return bytes(header)


def fake_macho(machine: int) -> bytes:
    return b"\xcf\xfa\xed\xfe" + struct.pack("<I", machine)


def fake_pe(machine: int) -> bytes:
    header = bytearray(70)
    header[:2] = b"MZ"
    struct.pack_into("<I", header, 60, 64)
    header[64:68] = b"PE\0\0"
    struct.pack_into("<H", header, 68, machine)
    return bytes(header)


if __name__ == "__main__":
    unittest.main()
