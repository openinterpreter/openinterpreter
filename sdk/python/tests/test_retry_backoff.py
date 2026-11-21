from __future__ import annotations

import pytest

import openai_codex.retry as retry_module
from openai_codex.errors import ServerBusyError
from openai_codex.retry import retry_on_overload


@pytest.mark.parametrize(
    ("initial_delay_s", "max_delay_s", "max_attempts"),
    [
        (0.25, 2.0, 4),
        (2.0, 2.0, 2),
        (8.0, 2.0, 3),
        (0.5, 1.0, 5),
    ],
)
@pytest.mark.parametrize("jitter_edge", ["low", "high"])
def test_backoff_sleeps_stay_within_max_delay(
    monkeypatch: pytest.MonkeyPatch,
    initial_delay_s: float,
    max_delay_s: float,
    max_attempts: int,
    jitter_edge: str,
) -> None:
    sleeps: list[float] = []
    monkeypatch.setattr(retry_module.time, "sleep", sleeps.append)
    monkeypatch.setattr(
        retry_module.random,
        "uniform",
        lambda low, high: low if jitter_edge == "low" else high,
    )

    def always_busy() -> None:
        raise ServerBusyError(-32001, "in-process app-server request queue is full")

    with pytest.raises(ServerBusyError):
        retry_on_overload(
            always_busy,
            max_attempts=max_attempts,
            initial_delay_s=initial_delay_s,
            max_delay_s=max_delay_s,
        )

    assert len(sleeps) == max_attempts - 1
    assert max(sleeps) <= max_delay_s
    assert min(sleeps) >= 0.0
