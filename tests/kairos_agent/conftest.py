"""Shared fixtures for kairos_agent tests.

Everything here is host-agnostic: tmp storage path, fake clock, dummy LLM.
No imports from `interpreter.*` — Kairos is standalone.
"""
from __future__ import annotations

from datetime import datetime, timedelta
from pathlib import Path

import pytest

from kairos_agent import (
    Coordinator,
    DummyLLMClient,
    KairosConfig,
    MemoryManager,
    configure_logging,
)
from kairos_agent.common.interfaces import Clock
from kairos_agent.common.paths import ensure_storage_dir


def pytest_configure(config: pytest.Config) -> None:  # noqa: D401
    config.addinivalue_line(
        "markers",
        "llm: tests that hit a real LLM provider (skipped by default)",
    )


def pytest_collection_modifyitems(config: pytest.Config, items: list[pytest.Item]) -> None:
    if config.getoption("-m") and "llm" in config.getoption("-m"):
        return  # user explicitly asked for them
    skip_marker = pytest.mark.skip(reason="needs -m llm")
    for item in items:
        if "llm" in item.keywords:
            item.add_marker(skip_marker)


class FakeClock(Clock):
    """Hand-cranked clock so tests don't depend on wall time."""

    def __init__(self, start: datetime | None = None):
        self._now = start or datetime(2030, 1, 1, 0, 0, 0)

    def now(self) -> datetime:
        return self._now

    def advance(self, seconds: float) -> None:
        self._now = self._now + timedelta(seconds=seconds)


@pytest.fixture
def fake_clock() -> FakeClock:
    return FakeClock()


@pytest.fixture
def storage_path(tmp_path: Path) -> Path:
    return ensure_storage_dir(tmp_path / "kairos")


@pytest.fixture
def dummy_llm() -> DummyLLMClient:
    return DummyLLMClient()


@pytest.fixture
def config(storage_path: Path, fake_clock: FakeClock,
           dummy_llm: DummyLLMClient) -> KairosConfig:
    cfg = KairosConfig(
        storage_path=storage_path,
        tick_interval=0.05,
        clock=fake_clock,
        llm_client=dummy_llm,
    )
    configure_logging(cfg.storage_path)
    return cfg


@pytest.fixture
def memory(config: KairosConfig):
    mem = MemoryManager(config)
    yield mem
    mem.close()


@pytest.fixture
def coordinator() -> Coordinator:
    coord = Coordinator()
    yield coord
    coord.stop()
