"""Filesystem helpers for Kairos storage."""
from __future__ import annotations

import os
from pathlib import Path


DEFAULT_STORAGE_DIR = Path.home() / ".kairos_agent"


def ensure_storage_dir(path: str | os.PathLike | None = None) -> Path:
    """Resolve a storage directory, create it if needed, return the Path.

    If `path` is None, falls back to ~/.kairos_agent. The `logs/` subdirectory
    is also created so the logger can write immediately.
    """
    target = Path(path) if path is not None else DEFAULT_STORAGE_DIR
    target = target.expanduser().resolve()
    (target / "logs").mkdir(parents=True, exist_ok=True)
    return target
