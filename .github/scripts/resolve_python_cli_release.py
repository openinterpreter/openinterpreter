#!/usr/bin/env python3
"""Bind downstream Python publication to a published stable CLI release."""

import argparse
import itertools
import json
import re
import subprocess
from pathlib import Path


def github_api(path: str):
    return json.loads(subprocess.check_output(["gh", "api", path], text=True))


def github_items(path: str, key: str | None = None):
    separator = "&" if "?" in path else "?"
    for page in itertools.count(1):
        response = github_api(f"{path}{separator}per_page=100&page={page}")
        items = response[key] if key else response
        yield from items
        if len(items) < 100:
            return


def resolve_release(
    repository: str, run_id: str, *, run: dict | None = None
) -> dict[str, str] | None:
    if repository != "openai/codex" or not re.fullmatch(r"[1-9][0-9]*", run_id):
        raise ValueError("Expected an openai/codex Rust release run ID")
    prefix = f"repos/{repository}"
    if run is None:
        run = github_api(f"{prefix}/actions/runs/{run_id}")
    if (
        str(run["id"]) != run_id
        or run["path"] != ".github/workflows/rust-release.yml"
        or run["event"] != "push"
        or run["status"] != "completed"
        or run["head_repository"]["full_name"] != repository
    ):
        raise ValueError("Python publication requires a completed Rust release run")
    tag = run["head_branch"]
    if not isinstance(tag, str) or not tag.startswith("rust-v"):
        raise ValueError("The Rust release run must identify a release tag")
    match = re.fullmatch(r"rust-v([0-9]+\.[0-9]+\.[0-9]+)", tag)
    if match is None:
        return None  # CLI prereleases do not publish the Python SDK.
    revision = run["head_sha"]
    if not re.fullmatch(r"[0-9a-f]{40}", revision):
        raise ValueError("The Rust release run must identify an exact commit")

    # A partial rerun of an unrelated publisher may not contain the release job.
    # Inspect all attempts and use the last release job that actually ran.
    release_job = max(
        (
            job
            for job in github_items(
                f"{prefix}/actions/runs/{run_id}/jobs?filter=all", "jobs"
            )
            if job["name"] == "release" and job["conclusion"] != "skipped"
        ),
        key=lambda job: (job["run_attempt"], job["id"]),
        default=None,
    )
    if release_job is None or release_job["conclusion"] != "success":
        raise ValueError("Python publication requires a successful release job")

    target = github_api(f"{prefix}/git/ref/tags/{tag}")["object"]
    for _ in range(8):
        if target["type"] != "tag":
            break
        target = github_api(f"{prefix}/git/tags/{target['sha']}")["object"]
    if target.get("type") != "commit" or target.get("sha") != revision:
        raise ValueError("The release tag no longer matches the successful CLI run")
    release = github_api(f"{prefix}/releases/tags/{tag}")
    if release["draft"] or release["prerelease"] or release["tag_name"] != tag:
        raise ValueError("The CLI release must be published and stable")

    # The runtime builder downloads these six wheels and builds the two musl
    # wheels from the release's package archives.
    required_assets = {
        f"openai_codex_cli_bin-{match[1]}-py3-none-{platform}.whl"
        for platform in (
            "macosx_10_9_x86_64",
            "macosx_11_0_arm64",
            "manylinux_2_17_aarch64",
            "manylinux_2_17_x86_64",
            "win_amd64",
            "win_arm64",
        )
    } | {
        f"codex-package-{arch}-unknown-linux-musl.tar.gz"
        for arch in ("aarch64", "x86_64")
    }
    available_assets = {
        asset["name"]
        for asset in github_items(f"{prefix}/releases/{release['id']}/assets")
        if asset["state"] == "uploaded" and asset["size"] > 0
    }
    if missing := required_assets - available_assets:
        raise ValueError(f"The CLI release is missing Python inputs: {sorted(missing)}")
    return {"source_sha": revision, "release_tag": tag, "version": match[1]}


def main(argv: list[str] | None = None) -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("run_id")
    parser.add_argument("--repository", required=True)
    parser.add_argument("--event-path", type=Path)
    parser.add_argument("--github-output", type=Path, required=True)
    args = parser.parse_args(argv)
    event = json.loads(args.event_path.read_text()) if args.event_path else {}
    release = resolve_release(
        args.repository, args.run_id, run=event.get("workflow_run")
    )
    with args.github_output.open("a") as output:
        print(f"publish={str(release is not None).lower()}", file=output)
        for name, value in (release or {}).items():
            print(f"{name}={value}", file=output)


if __name__ == "__main__":
    main()
