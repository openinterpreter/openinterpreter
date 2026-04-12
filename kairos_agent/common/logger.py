"""Structured JSON-line logging for kairos_agent.

One file handler shared by all submodules under <storage_path>/logs/kairos.log,
plus an optional console handler. Each record is one JSON object per line so
logs can be tailed and parsed by simple tooling.
"""
from __future__ import annotations

import json
import logging
import sys
from pathlib import Path
from typing import Any

_CONFIGURED = False
_LOG_FILE: Path | None = None

# Reserved attribute names used internally by LogRecord. Anything in the user's
# `extra=` mapping that collides with one of these would otherwise raise
# KeyError inside logging.makeRecord. We rename collisions on the way in so
# call sites don't have to know the list.
_RESERVED_RECORD_FIELDS = frozenset({
    "name", "msg", "args", "levelname", "levelno", "pathname", "filename",
    "module", "exc_info", "exc_text", "stack_info", "lineno", "funcName",
    "created", "msecs", "relativeCreated", "thread", "threadName",
    "processName", "process", "message", "asctime",
})


def _sanitize_extra(extra: dict[str, Any] | None) -> dict[str, Any] | None:
    if not extra:
        return extra
    safe: dict[str, Any] = {}
    for key, value in extra.items():
        if key in _RESERVED_RECORD_FIELDS:
            safe[f"{key}_"] = value
        else:
            safe[key] = value
    return safe


class _JsonLineFormatter(logging.Formatter):
    def format(self, record: logging.LogRecord) -> str:
        payload: dict[str, Any] = {
            "ts": self.formatTime(record, "%Y-%m-%dT%H:%M:%S"),
            "level": record.levelname,
            "logger": record.name,
            "msg": record.getMessage(),
        }
        # Include any structured extras attached via logger.info(..., extra={...}).
        for key, value in record.__dict__.items():
            if key in ("args", "msg", "levelname", "levelno", "pathname", "filename",
                       "module", "exc_info", "exc_text", "stack_info", "lineno",
                       "funcName", "created", "msecs", "relativeCreated", "thread",
                       "threadName", "processName", "process", "name", "message",
                       "asctime"):
                continue
            try:
                json.dumps(value)
                payload[key] = value
            except TypeError:
                payload[key] = repr(value)
        if record.exc_info:
            payload["exc"] = self.formatException(record.exc_info)
        return json.dumps(payload, ensure_ascii=False)


def configure_logging(storage_dir: Path, level: int = logging.INFO,
                      console: bool = False) -> Path:
    """Set up the kairos_agent logger hierarchy. Idempotent.

    Returns the path to the log file so callers can surface it to the user.
    """
    global _CONFIGURED, _LOG_FILE
    log_file = storage_dir / "logs" / "kairos.log"
    if _CONFIGURED and _LOG_FILE == log_file:
        return log_file

    root = logging.getLogger("kairos_agent")
    root.setLevel(level)
    # Clear any prior handlers if reconfiguring (e.g. tests with new tmp dir).
    for handler in list(root.handlers):
        root.removeHandler(handler)

    formatter = _JsonLineFormatter()

    file_handler = logging.FileHandler(log_file, encoding="utf-8")
    file_handler.setFormatter(formatter)
    root.addHandler(file_handler)

    if console:
        console_handler = logging.StreamHandler(sys.stderr)
        console_handler.setFormatter(formatter)
        root.addHandler(console_handler)

    root.propagate = False
    _CONFIGURED = True
    _LOG_FILE = log_file
    return log_file


class _SafeLogger(logging.Logger):
    """Logger that sanitizes `extra=` keys to avoid LogRecord field collisions."""

    def _log(self, level, msg, args, exc_info=None, extra=None,  # type: ignore[override]
             stack_info=False, stacklevel=1):
        super()._log(level, msg, args, exc_info=exc_info,
                     extra=_sanitize_extra(extra), stack_info=stack_info,
                     stacklevel=stacklevel)


def get_logger(name: str) -> logging.Logger:
    """Return a child logger of the kairos_agent root.

    Pass the submodule name without the prefix, e.g. get_logger("memory").
    Calling before configure_logging() still works — records will buffer until
    a handler is attached.
    """
    full_name = f"kairos_agent.{name}"
    logger = logging.getLogger(full_name)
    # Force the safe subclass on every kairos_agent.* logger so any caller
    # benefits from extras sanitization.
    logger.__class__ = _SafeLogger
    return logger
