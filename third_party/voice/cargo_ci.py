"""Stage receipt-checked voice build inputs for Cargo and configure a shard."""

import argparse
import json
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))

from release_runtime import stage as stage_runtime
from runtime import digest
from sdk import MODULES


def read_receipt(path: Path, target: str, commit: str) -> dict:
    receipt = json.loads(path.read_text())
    if (
        receipt["target"] != target
        or receipt["sourceCommit"] != commit
        or receipt["sourceManifestSha256"]
        != digest(Path(__file__).with_name("sources.json"))
    ):
        raise ValueError("native inputs do not match target, commit and sources")
    return receipt


def stage(sdk: Path, runtime: Path, output: Path, target: str, commit: str) -> None:
    sdk, runtime, output = (
        sdk.resolve(strict=True),
        runtime.resolve(strict=True),
        output.resolve(),
    )
    if any(output.is_relative_to(source) for source in (sdk, runtime)):
        raise ValueError("output must be outside the native inputs")
    files = read_receipt(sdk / "sdk.json", target, commit)["files"]
    native = read_receipt(runtime / "runtime.json", target, commit)
    library_dir = output / "runtime" / ("bin" if target.endswith("msvc") else "lib")
    prepared = {
        entry.get("sourceSha256"): runtime / entry["path"]
        for entry in native["libraries"]
        if Path(entry["path"]).parent.name == library_dir.name
    }

    output.mkdir(parents=True)
    try:
        stage_runtime(runtime, output / "runtime", target)
        for entry in files:
            source = (sdk / entry["path"]).resolve(strict=True)
            relative = source.relative_to(sdk)
            if digest(source) != entry["sha256"]:
                raise ValueError(f"SDK file does not match its receipt: {relative}")
            dest = output / "sdk" / relative
            dest.parent.mkdir(parents=True, exist_ok=True)
            if source.suffix == ".pc":
                # Some transitive libraries retain the producer's absolute prefix.
                dest.write_text(
                    re.sub(
                        r"^prefix=.*$",
                        "prefix=${pcfiledir}/../..",
                        source.read_text(),
                        flags=re.MULTILINE,
                    )
                )
            else:
                shutil.copy2(source, dest)

            # Link with the prepared runtime bytes and put MSVC import libraries
            # beside their DLLs so nextest collects a self-contained search path.
            if relative.parent == Path("lib"):
                alias = library_dir / relative.name
                payload = (
                    source if source.suffix == ".lib" else prepared.get(entry["sha256"])
                )
                if payload and not alias.exists():
                    shutil.copy2(payload, alias)
    except BaseException:
        # Windows cannot delete read-only files copied from Bazel.
        if os.name == "nt":
            for copied in output.rglob("*"):
                copied.chmod(copied.stat().st_mode | stat.S_IWRITE)
        shutil.rmtree(output)
        raise


def environment(root: Path, target: str, pkg_config: Path) -> dict[str, str]:
    root, pkg_config = root.resolve(strict=True), pkg_config.resolve(strict=True)
    read_receipt(root / "runtime/runtime.json", target, os.environ["GITHUB_SHA"])
    suffix = target.replace("-", "_")
    values = {
        f"PKG_CONFIG_{suffix}": str(pkg_config),
        f"PKG_CONFIG_PATH_{suffix}": str(root / "sdk/lib/pkgconfig"),
        "CODEX_TEST_VOICE_RUNTIME": str(root / "runtime"),
    }
    for module in MODULES:
        key = re.sub(r"[^A-Z0-9]", "_", module.upper())
        values[f"SYSTEM_DEPS_{key}_SEARCH_NATIVE"] = str(
            root / "runtime" / ("bin" if target.endswith("msvc") else "lib")
        )
        if module.startswith("gstreamer"):
            values[f"SYSTEM_DEPS_{key}_LDFLAGS"] = ""
    if not target.endswith("windows-msvc"):
        origin = "@loader_path" if target.endswith("darwin") else "$ORIGIN"
        values[f"CARGO_TARGET_{suffix.upper()}_RUSTFLAGS"] = (
            f"-C link-arg=-Wl,-rpath,{origin}/../lib"
        )
    return values


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    producer = sub.add_parser("stage")
    producer.add_argument("--sdk", type=Path, required=True)
    producer.add_argument("--runtime", type=Path, required=True)
    producer.add_argument("--output", type=Path, required=True)
    consumer = sub.add_parser("configure")
    consumer.add_argument("--root", type=Path, required=True)
    for command in (producer, consumer):
        command.add_argument("--target", required=True)
    args = parser.parse_args()
    if args.command == "stage":
        stage(
            args.sdk, args.runtime, args.output, args.target, os.environ["GITHUB_SHA"]
        )
        return
    pkg_config = os.environ.get("VOICE_PKG_CONFIG") or shutil.which("pkg-config")
    if not pkg_config:
        parser.error("pkg-config is required on the build host")
    values = environment(args.root, args.target, Path(pkg_config))
    # Probe only the staged SDK so a missing native module cannot resolve from
    # the host. Cargo keeps its usual system search for unrelated dependencies.
    probe_env = {
        **os.environ,
        "PKG_CONFIG_PATH": "",
        "PKG_CONFIG_LIBDIR": str(args.root.resolve() / "sdk/lib/pkgconfig"),
    }
    subprocess.run(
        [pkg_config, "--exists", *MODULES, "gstreamer-1.0 >= 1.28"],
        env=probe_env,
        check=True,
    )
    with open(os.environ["GITHUB_ENV"], "a", encoding="utf-8") as output:
        for key, value in values.items():
            output.write(f"{key}={value}\n")


if __name__ == "__main__":
    main()
