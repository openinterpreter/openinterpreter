"""Real LLM integration tests — skipped by default.

Run explicitly with:  pytest tests/kairos_agent/ -m llm -v
Requires OPENAI_API_KEY (or another provider env var) to be set.
"""
from __future__ import annotations

import pytest

pytestmark = pytest.mark.llm


def test_placeholder() -> None:
    """Placeholder so the module is importable. Replace with real LLM tests."""
    pytest.skip("real LLM tests not yet wired — add provider adapter first")
