"""Normalizer: PII redaction + format_code passthrough."""
from __future__ import annotations

from kairos_agent import format_code, redact_pii


def test_redact_email() -> None:
    assert "<email>" in redact_pii("contact me at alice@example.com please")


def test_redact_ip() -> None:
    assert "<ip>" in redact_pii("server is at 192.168.1.42 ok")


def test_redact_absolute_path() -> None:
    assert "<path>" in redact_pii("file at /home/user/secret/data.json")


def test_redact_home_path() -> None:
    assert "<path>" in redact_pii("config in ~/my_app/config.yaml")


def test_redact_preserves_safe_text() -> None:
    safe = "This is perfectly fine text with no PII."
    assert redact_pii(safe) == safe


def test_redact_empty() -> None:
    assert redact_pii("") == ""


def test_format_code_passthrough() -> None:
    code = "def foo():\n    return 42\n"
    assert format_code(code) == code
