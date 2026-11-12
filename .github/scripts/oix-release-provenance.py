#!/usr/bin/env python3
"""Verify original tag-build attestations before publishing Linux runtime packages.

The release job creates and attests a deterministic manifest; publish-existing
must consume those original bytes, never create a substitute attestation.
"""

import argparse
import hashlib
import json
import pathlib
import re
import subprocess
import sys

TARGETS = ("aarch64-unknown-linux-musl", "x86_64-unknown-linux-musl")
SUFFIXES = ("tar.gz", "tar.zst")
MANIFEST = "open-interpreter-package_SHA256SUMS"


def checked(condition, message):
    if not condition:
        raise ValueError(message)


def asset(dist, name):
    matches = list(dist.rglob(name))
    checked(
        len(matches) == 1 and matches[0].is_file(),
        f"expected exactly one file named {name}, found {len(matches)}",
    )
    return matches[0]


def verify_attestation(path, args):
    result = subprocess.run(
        [
            "gh",
            "attestation",
            "verify",
            str(path),
            "--repo",
            args.repo,
            "--cert-identity",
            f"https://github.com/{args.repo}/.github/workflows/rust-release.yml@{args.ref}",
            "--source-ref",
            args.ref,
            "--source-digest",
            args.commit,
            "--format",
            "json",
        ],
        check=True,
        capture_output=True,
        text=True,
    )
    verified = json.loads(result.stdout)
    run_prefix = f"https://github.com/{args.repo}/actions/runs/{args.run_id}/attempts/"
    checked(
        isinstance(verified, list)
        and any(
            re.fullmatch(
                re.escape(run_prefix) + r"[1-9][0-9]*",
                item.get("verificationResult", {})
                .get("signature", {})
                .get("certificate", {})
                .get("runInvocationURI", ""),
            )
            for item in verified
            if isinstance(item, dict)
        ),
        f"no verified attestation from original build run {args.run_id} for {path.name}",
    )


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("mode", choices=("prepare", "verify"))
    parser.add_argument("--dist", type=pathlib.Path, required=True)
    parser.add_argument("--repo", required=True)
    parser.add_argument("--ref", required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--run-id", required=True)
    args = parser.parse_args()
    checked(
        re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", args.repo),
        "invalid repository",
    )
    checked(
        re.fullmatch(
            r"refs/tags/rust-v[0-9]+\.[0-9]+\.[0-9]+(?:-[A-Za-z0-9.]+)?", args.ref
        ),
        "not a release tag",
    )
    checked(re.fullmatch(r"[0-9a-f]{40}", args.commit), "invalid source commit")
    checked(re.fullmatch(r"[0-9]+", args.run_id), "invalid original run ID")

    lines = []
    for target in TARGETS:
        for suffix in SUFFIXES:
            name = f"open-interpreter-package-{target}.{suffix}"
            path = asset(args.dist, name)
            verify_attestation(path, args)
            with path.open("rb") as archive:
                digest = hashlib.file_digest(archive, "sha256").hexdigest()
            lines.append(f"{digest}  {name}\n")
    expected = "".join(lines).encode("ascii")
    manifest = (
        asset(args.dist, MANIFEST) if args.mode == "verify" else args.dist / MANIFEST
    )
    if args.mode == "prepare":
        checked(not manifest.exists(), "refusing to overwrite checksum manifest")
        manifest.write_bytes(expected)
    else:
        verify_attestation(manifest, args)
        checked(
            manifest.read_bytes() == expected,
            "original attested manifest does not match archives",
        )
    print(
        f"Verified four Linux archives and original checksum manifest for {args.ref} ({args.commit})"
    )


if __name__ == "__main__":
    try:
        main()
    except (
        ValueError,
        OSError,
        subprocess.CalledProcessError,
        json.JSONDecodeError,
    ) as exc:
        sys.exit(f"release provenance verification failed: {exc}")
