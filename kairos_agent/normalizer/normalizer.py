"""Style + privacy helpers (MVP stub).

Two pure functions: redact common PII patterns from a string, and format code
(passthrough at MVP — `black` integration is a future iteration). These are
not yet wired into the rest of Kairos; once everything else is stable they'll
become an optional middleware layer in front of the LLMClient.
"""
from __future__ import annotations

import re


_EMAIL_RE = re.compile(r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b")
_IPV4_RE = re.compile(r"\b(?:\d{1,3}\.){3}\d{1,3}\b")
_ABS_PATH_RE = re.compile(r"(?:/[^\s/]+){2,}")
_HOME_PATH_RE = re.compile(r"~/[^\s]+")


def redact_pii(text: str) -> str:
    """Replace common PII tokens with placeholders.

    Conservative on purpose — only replaces patterns that are unambiguous.
    """
    if not text:
        return text
    out = _EMAIL_RE.sub("<email>", text)
    out = _IPV4_RE.sub("<ip>", out)
    out = _HOME_PATH_RE.sub("<path>", out)
    out = _ABS_PATH_RE.sub("<path>", out)
    return out


def format_code(text: str, language: str = "python") -> str:
    """Format a code snippet. MVP passthrough — keeps the seam for later."""
    return text
