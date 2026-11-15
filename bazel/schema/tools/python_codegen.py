"""Generate SDK build outputs in scratch space without formatting."""

import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path


def main() -> None:
    files = json.loads(Path(sys.argv[1]).read_text())
    schema = Path(sys.argv[2]).resolve()
    output = Path(sys.argv[3]).resolve()
    with tempfile.TemporaryDirectory() as temporary:
        sdk = Path(temporary) / "sdk/python"
        for source, relative in files:
            destination = sdk / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source, destination)

        environment = {
            **os.environ,
            "PYTHONPATH": os.pathsep.join(sys.path),
        }
        subprocess.run(
            [
                sys.executable,
                str(sdk / "scripts/update_sdk_artifacts.py"),
                "generate-types",
                "--no-format",
                "--schema-dir",
                str(schema),
            ],
            cwd=sdk,
            env=environment,
            check=True,
        )
        for relative in (
            "src/openai_codex/api.py",
            "src/openai_codex/generated/v2_all.py",
            "src/openai_codex/generated/notification_registry.py",
        ):
            destination = output / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(sdk / relative, destination)


if __name__ == "__main__":
    main()
