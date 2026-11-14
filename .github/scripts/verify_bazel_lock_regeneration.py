#!/usr/bin/env python3
"""Package a review-only Bazel lockfile with exact source/run provenance."""

import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess


LOCK = "MODULE.bazel.lock"


def git(*args: str) -> bytes:
    return subprocess.check_output(["git", *args], stderr=subprocess.PIPE)


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def package(source_sha: str, run_id: int, run_attempt: int, output_dir: Path) -> dict:
    if not re.fullmatch(r"[0-9a-f]{40}", source_sha):
        raise ValueError("source_sha must be a full lowercase SHA-1")
    if run_id <= 0 or run_attempt <= 0:
        raise ValueError("run identity must be positive")
    actual = git("rev-parse", "HEAD").decode().strip()
    if actual != source_sha:
        raise ValueError("checkout HEAD differs from requested source")
    changed = git("diff", "--name-only", "--no-ext-diff").decode().splitlines()
    if changed != [LOCK]:
        raise ValueError(f"expected only a changed {LOCK}; got {changed!r}")
    if git("ls-files", "--others", "--exclude-standard").strip():
        raise ValueError("unexpected untracked source files")

    original = git("show", f"{source_sha}:{LOCK}")
    candidate = Path(LOCK).read_bytes()
    if not candidate or original == candidate:
        raise ValueError("lock candidate is empty or unchanged")
    if output_dir.exists():
        raise ValueError("review output directory already exists")
    output_dir.mkdir(parents=True)
    shutil.copyfile(LOCK, output_dir / LOCK)
    manifest = {
        "schema": "oix-bazel-lock-review/v1",
        "repository": "openinterpreter/openinterpreter",
        "source_sha": source_sha,
        "source_tree": git("rev-parse", "HEAD^{tree}").decode().strip(),
        "run_id": run_id,
        "run_attempt": run_attempt,
        "command": "mod deps --lockfile_mode=update; mod deps --lockfile_mode=error",
        "original_lock_sha256": sha256(original),
        "candidate_lock_sha256": sha256(candidate),
        "candidate_lock_bytes": len(candidate),
    }
    (output_dir / "provenance.json").write_text(
        json.dumps(manifest, sort_keys=True, indent=2) + "\n", encoding="utf-8"
    )
    print(json.dumps(manifest, sort_keys=True))
    return manifest


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-sha", required=True)
    parser.add_argument("--run-id", required=True, type=int)
    parser.add_argument("--run-attempt", required=True, type=int)
    parser.add_argument("--output-dir", required=True, type=Path)
    args = parser.parse_args()
    package(args.source_sha, args.run_id, args.run_attempt, args.output_dir)


if __name__ == "__main__":
    main()
