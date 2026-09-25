"""Runtime version and checkout-schema requirements for newer SDK options."""

import json
import re
import subprocess
from dataclasses import dataclass
from functools import cached_property
from pathlib import Path
from tempfile import TemporaryDirectory

from packaging.version import InvalidVersion, Version

MINIMUM_RUNTIME_VERSION = "0.151.0"


def require_runtime_version(version: str | None) -> None:
    """Reject unknown or unsupported versions, including prereleases at the minimum."""
    # CLI alpha hotfixes use 0.154.0-alpha.1.2; PEP 440 spells that a1.post2.
    normalized = re.sub(r"-alpha\.(\d+)\.(\d+)$", r"a\1.post\2", version or "")
    try:
        if Version(normalized) >= Version(MINIMUM_RUNTIME_VERSION):
            return
    except InvalidVersion:
        pass
    raise ValueError(
        f"Codex CLI {MINIMUM_RUNTIME_VERSION} or newer is required; "
        f"reported version is {version or 'unknown'!r}"
    )


@dataclass
class CheckoutCapabilities:
    """Lazily inspect the same executable and configuration as a running checkout."""

    command: tuple[str, ...]
    cwd: str | None
    env: dict[str, str]

    @cached_property
    def fields(self) -> dict[str, frozenset[str]]:
        try:
            with TemporaryDirectory(prefix="codex-sdk-schema-") as directory:
                subprocess.run(
                    [*self.command, "generate-json-schema", "--experimental", "--out", directory],
                    cwd=self.cwd,
                    env=self.env,
                    capture_output=True,
                    check=True,
                    timeout=30,
                )
                result = {}
                for method, name in (
                    ("turn/start", "TurnStartParams"),
                    ("thread/resume", "ThreadResumeParams"),
                    ("thread/fork", "ThreadForkParams"),
                ):
                    schema = json.loads((Path(directory) / "v2" / f"{name}.json").read_text())
                    properties = schema.get("properties") if isinstance(schema, dict) else None
                    if not isinstance(properties, dict):
                        raise ValueError(f"Missing properties in {name} schema")
                    result[method] = frozenset(properties)
                return result
        except (OSError, subprocess.SubprocessError, ValueError) as exc:
            raise ValueError("Could not inspect the unversioned CLI's experimental schema") from exc
