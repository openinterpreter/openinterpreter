"""Canonical Codex package directory layout."""

import hashlib
import json
import re
import shutil
import stat
import struct
from pathlib import Path

from .targets import PackageInputs
from .targets import PackageVariant
from .targets import TargetSpec
from .zsh import ZSH_RESOURCE_PATH


LAYOUT_VERSION = 1
LLAMA_CPP_RESOURCE_PATH = Path("llama.cpp")


def verify_llama_server_input(
    binary_path: Path | None, expected: str | None, spec: TargetSpec
) -> None:
    if binary_path is None:
        raise RuntimeError("open-interpreter packages require a llama-server binary.")
    if expected is None or re.fullmatch(r"[0-9a-fA-F]{64}", expected) is None:
        raise RuntimeError("open-interpreter packages require a pinned llama-server SHA-256.")
    with binary_path.open("rb") as binary:
        actual = hashlib.file_digest(binary, "sha256").hexdigest()
    if actual != expected.lower():
        raise RuntimeError("llama-server binary SHA-256 does not match the pinned digest.")
    with binary_path.open("rb") as binary:
        header = binary.read(64)
        if spec.is_linux:
            if not header.startswith(b"\x7fELF") or len(header) < 20:
                raise RuntimeError("llama-server is not an ELF executable for this target.")
            if header[4:6] != b"\x02\x01":
                raise RuntimeError("llama-server requires a little-endian 64-bit ELF executable.")
            machine = struct.unpack_from("<H", header, 18)[0]
            expected_machine = 183 if spec.target.startswith("aarch64-") else 62
        elif spec.is_windows:
            if not header.startswith(b"MZ") or len(header) < 64:
                raise RuntimeError("llama-server is not a Windows PE executable.")
            pe_offset = struct.unpack_from("<I", header, 60)[0]
            if pe_offset > 1_048_576:
                raise RuntimeError("llama-server has an invalid PE header offset.")
            binary.seek(pe_offset)
            pe_header = binary.read(6)
            if not pe_header.startswith(b"PE\0\0") or len(pe_header) < 6:
                raise RuntimeError("llama-server has an invalid PE header.")
            machine = struct.unpack_from("<H", pe_header, 4)[0]
            expected_machine = 0xAA64 if spec.target.startswith("aarch64-") else 0x8664
        else:
            if header[:4] != b"\xcf\xfa\xed\xfe" or len(header) < 8:
                raise RuntimeError("llama-server requires a thin 64-bit macOS Mach-O executable.")
            machine = struct.unpack_from("<I", header, 4)[0]
            expected_machine = 0x0100000C if spec.target.startswith("aarch64-") else 0x01000007
        if machine != expected_machine:
            raise RuntimeError(f"llama-server CPU architecture does not match {spec.target}.")


def prepare_package_dir(package_dir: Path, *, force: bool) -> None:
    if package_dir.exists():
        if not package_dir.is_dir():
            raise RuntimeError(
                f"Package output exists and is not a directory: {package_dir}"
            )
        if any(package_dir.iterdir()):
            if not force:
                raise RuntimeError(
                    f"Package output directory is not empty: {package_dir}. "
                    "Pass --force to replace it."
                )
            shutil.rmtree(package_dir)

    package_dir.mkdir(parents=True, exist_ok=True)


def build_package_dir(
    package_dir: Path,
    version: str,
    variant: PackageVariant,
    spec: TargetSpec,
    inputs: PackageInputs,
) -> None:
    if variant.llama_cpp_required:
        verify_llama_server_input(inputs.llama_server_bin, inputs.llama_server_sha256, spec)
    bin_dir = package_dir / "bin"
    resources_dir = package_dir / "codex-resources"
    path_dir = package_dir / "codex-path"
    bin_dir.mkdir()
    resources_dir.mkdir()
    path_dir.mkdir()

    entrypoint_name = variant.entrypoint_name(spec)
    copy_executable(
        inputs.entrypoint_bin,
        bin_dir / entrypoint_name,
        is_windows=spec.is_windows,
    )
    for alias_name in variant.alias_names(spec):
        copy_executable(
            inputs.entrypoint_bin,
            bin_dir / alias_name,
            is_windows=spec.is_windows,
        )
    if variant.managed_codex_required:
        if inputs.managed_codex_bin is None:
            raise RuntimeError(
                f"{variant.name} packages require a managed Codex binary."
            )
        copy_executable(
            inputs.managed_codex_bin,
            bin_dir / variant.managed_codex_name(spec),
            is_windows=spec.is_windows,
        )
    copy_executable(
        inputs.code_mode_host_bin,
        bin_dir / f"codex-code-mode-host{spec.exe_suffix}",
        is_windows=spec.is_windows,
    )
    copy_executable(inputs.rg_bin, path_dir / spec.rg_name, is_windows=spec.is_windows)

    if inputs.zsh_bin is not None:
        copy_executable(
            inputs.zsh_bin,
            resources_dir / ZSH_RESOURCE_PATH,
            is_windows=False,
        )

    if inputs.bwrap_bin is not None:
        copy_executable(inputs.bwrap_bin, resources_dir / "bwrap", is_windows=False)

    if inputs.codex_command_runner_bin is not None:
        copy_executable(
            inputs.codex_command_runner_bin,
            resources_dir / "codex-command-runner.exe",
            is_windows=True,
        )

    if inputs.codex_windows_sandbox_setup_bin is not None:
        copy_executable(
            inputs.codex_windows_sandbox_setup_bin,
            resources_dir / "codex-windows-sandbox-setup.exe",
            is_windows=True,
        )

    if variant.llama_cpp_required:
        copy_executable(
            inputs.llama_server_bin,
            resources_dir / LLAMA_CPP_RESOURCE_PATH / spec.llama_server_name,
            is_windows=spec.is_windows,
        )
    elif inputs.llama_server_bin is not None:
        raise RuntimeError(f"llama-server is not supported for {variant.name} packages.")

    metadata = {
        "layoutVersion": LAYOUT_VERSION,
        "version": version,
        "target": spec.target,
        "variant": variant.name,
        "entrypoint": f"bin/{entrypoint_name}",
        "resourcesDir": "codex-resources",
        "pathDir": "codex-path",
    }
    if variant.managed_codex_required:
        metadata["managedCodex"] = f"bin/{variant.managed_codex_name(spec)}"
    if variant.llama_cpp_required:
        metadata["llamaServer"] = str(
            Path("codex-resources") / LLAMA_CPP_RESOURCE_PATH / spec.llama_server_name
        )
        metadata["llamaServerSha256"] = inputs.llama_server_sha256.lower()
    write_json(package_dir / "codex-package.json", metadata)


def validate_package_dir(
    package_dir: Path,
    variant: PackageVariant,
    spec: TargetSpec,
    *,
    include_zsh: bool,
) -> None:
    required_dirs = [
        Path("bin"),
        Path("codex-resources"),
        Path("codex-path"),
    ]
    for relative_dir in required_dirs:
        path = package_dir / relative_dir
        if not path.is_dir():
            raise RuntimeError(f"Missing package directory: {relative_dir}")

    metadata_path = package_dir / "codex-package.json"
    if not metadata_path.is_file():
        raise RuntimeError("Missing package metadata: codex-package.json")

    with open(metadata_path, encoding="utf-8") as fh:
        metadata = json.load(fh)

    expected_metadata = {
        "layoutVersion": LAYOUT_VERSION,
        "target": spec.target,
        "variant": variant.name,
        "entrypoint": f"bin/{variant.entrypoint_name(spec)}",
        "resourcesDir": "codex-resources",
        "pathDir": "codex-path",
    }
    for key, expected in expected_metadata.items():
        actual = metadata.get(key)
        if actual != expected:
            raise RuntimeError(
                f"Invalid package metadata field {key!r}: expected {expected!r}, got {actual!r}"
            )

    required_files = [
        Path("bin") / variant.entrypoint_name(spec),
        Path("bin") / f"codex-code-mode-host{spec.exe_suffix}",
        Path("codex-path") / spec.rg_name,
    ]
    executable_files = list(required_files)
    for alias_name in variant.alias_names(spec):
        alias_path = Path("bin") / alias_name
        required_files.append(alias_path)
        executable_files.append(alias_path)

    if variant.managed_codex_required:
        managed_codex_path = Path("bin") / variant.managed_codex_name(spec)
        required_files.append(managed_codex_path)
        executable_files.append(managed_codex_path)

    if variant.llama_cpp_required:
        llama_server_path = (
            Path("codex-resources") / LLAMA_CPP_RESOURCE_PATH / spec.llama_server_name
        )
        required_files.append(llama_server_path)
        executable_files.append(llama_server_path)

    if include_zsh:
        zsh_path = Path("codex-resources") / ZSH_RESOURCE_PATH
        required_files.append(zsh_path)
        executable_files.append(zsh_path)

    if spec.is_linux:
        required_files.append(Path("codex-resources") / "bwrap")
        executable_files.append(Path("codex-resources") / "bwrap")

    if spec.is_windows:
        required_files.extend(
            [
                Path("codex-resources") / "codex-command-runner.exe",
                Path("codex-resources") / "codex-windows-sandbox-setup.exe",
            ]
        )

    for relative_file in required_files:
        path = package_dir / relative_file
        if not path.is_file():
            raise RuntimeError(f"Missing package file: {relative_file}")

    if variant.llama_cpp_required:
        if metadata.get("llamaServer") != str(llama_server_path):
            raise RuntimeError("Invalid packaged llama-server path in metadata")
        verify_llama_server_input(
            package_dir / llama_server_path, metadata.get("llamaServerSha256"), spec
        )

    if not spec.is_windows:
        for relative_file in executable_files:
            path = package_dir / relative_file
            if not is_executable(path):
                raise RuntimeError(f"Package file is not executable: {relative_file}")


def copy_executable(src: Path, dest: Path, *, is_windows: bool) -> None:
    dest.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(src, dest)
    if not is_windows:
        mode = dest.stat().st_mode
        dest.chmod(mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)


def write_json(path: Path, value: object) -> None:
    with open(path, "w", encoding="utf-8") as out:
        json.dump(value, out, indent=2)
        out.write("\n")


def is_executable(path: Path) -> bool:
    return bool(path.stat().st_mode & stat.S_IXUSR)
