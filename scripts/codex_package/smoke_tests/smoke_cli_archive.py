"""Smoke-test the public command surface of an assembled CLI archive."""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import tarfile
import tempfile
from pathlib import Path


SHELLS = ("bash", "zsh", "fish", "powershell")


def run(entrypoint: Path, root: Path, environment: dict[str, str], *args: str) -> str:
    completed = subprocess.run(
        [str(entrypoint), *args],
        cwd=root,
        env=environment,
        check=True,
        capture_output=True,
        text=True,
        timeout=45,
    )
    return f"{completed.stdout}\n{completed.stderr}"


def assert_open_interpreter_surface(label: str, output: str) -> None:
    assert "codex" not in output.casefold(), f"{label} leaked Codex branding:\n{output}"


def smoke_open_interpreter(root: Path, entrypoint: Path) -> None:
    environment = dict(os.environ)
    environment["INTERPRETER_HOME"] = str(root / "interpreter-home")
    environment.pop("CODEX_HOME", None)

    root_help = run(entrypoint, root, environment, "--help")
    exec_help = run(entrypoint, root, environment, "exec", "--help")
    version = run(entrypoint, root, environment, "--version")
    for label, output in (
        ("root help", root_help),
        ("exec help", exec_help),
        ("version", version),
    ):
        assert_open_interpreter_surface(label, output)
    assert "--chat-completions" in root_help
    assert "--chat-completions" in exec_help
    assert version.strip().startswith("interpreter "), version

    for shell in SHELLS:
        completion = run(entrypoint, root, environment, "completion", shell)
        assert_open_interpreter_surface(f"{shell} completion", completion)
        assert "interpreter" in completion.casefold(), completion
        assert "--chat-completions" in completion, completion


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--archive", type=Path, required=True)
    parser.add_argument(
        "--product", choices=("open-interpreter",), required=True
    )
    args = parser.parse_args()

    with tempfile.TemporaryDirectory(prefix="cli-package-smoke-") as temp:
        root = Path(temp)
        with tarfile.open(args.archive, "r:gz") as archive:
            archive.extractall(root, filter="data")
        metadata = json.loads((root / "codex-package.json").read_text())
        assert metadata["variant"] == args.product, metadata
        entrypoint = root / metadata["entrypoint"]
        assert entrypoint.name in {"interpreter", "interpreter.exe"}, entrypoint
        smoke_open_interpreter(root, entrypoint)


if __name__ == "__main__":
    main()
