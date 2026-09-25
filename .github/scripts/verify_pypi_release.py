#!/usr/bin/env python3
"""Wait for every Python release artifact to become visible on PyPI."""

import argparse
import http.client
import json
import time
import urllib.request

from packaging.version import Version

RUNTIME_PLATFORM_TAGS = {
    "macosx_10_9_x86_64",
    "macosx_11_0_arm64",
    "manylinux_2_17_aarch64",
    "manylinux_2_17_x86_64",
    "musllinux_1_1_aarch64",
    "musllinux_1_1_x86_64",
    "win_amd64",
    "win_arm64",
}


def verify_release(package: str, version: str) -> None:
    version = str(Version(version))
    name = package.replace("-", "_")
    if package == "openai-codex-cli-bin":
        expected = {
            f"{name}-{version}-py3-none-{tag}.whl" for tag in RUNTIME_PLATFORM_TAGS
        }
    else:
        expected = {f"{name}-{version}-py3-none-any.whl", f"{name}-{version}.tar.gz"}

    for attempt in range(30):
        try:
            with urllib.request.urlopen(
                f"https://pypi.org/pypi/{package}/{version}/json", timeout=30
            ) as response:
                data = json.load(response)
                if not isinstance(data, dict) or not isinstance(data.get("urls"), list):
                    raise ValueError("Expected a PyPI response with a urls list")
                if any(
                    not isinstance(file, dict)
                    or not isinstance(file.get("filename"), str)
                    for file in data["urls"]
                ):
                    raise ValueError("Expected each PyPI file to have a filename")
                actual = {file["filename"] for file in data["urls"]}
        except (
            OSError,
            http.client.HTTPException,
            ValueError,
        ) as error:
            print(f"Could not read {package} {version} from PyPI: {error}.")
        else:
            if actual == expected:
                print(f"All {package} {version} files are available on PyPI.")
                return
            print(f"Missing files: {sorted(expected - actual)}")
            print(f"Unexpected files: {sorted(actual - expected)}")
        if attempt < 29:
            time.sleep(10)

    raise SystemExit(f"{package} {version} files did not become available on PyPI.")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("package", choices=["openai-codex-cli-bin", "openai-codex"])
    parser.add_argument("version")
    args = parser.parse_args()
    verify_release(args.package, args.version)


if __name__ == "__main__":
    main()
