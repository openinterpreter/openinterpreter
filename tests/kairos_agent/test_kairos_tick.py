"""KairosCore tick loop + triggers."""
from __future__ import annotations

import threading
import time
from datetime import timedelta

from kairos_agent import (
    Coordinator,
    InactivityTrigger,
    KairosConfig,
    KairosCore,
    MemoryManager,
    TaskRequest,
    TimeTrigger,
)
from tests.kairos_agent.conftest import FakeClock


def test_tick_fires_time_trigger(config: KairosConfig, memory: MemoryManager,
                                 coordinator: Coordinator,
                                 fake_clock: FakeClock) -> None:
    calls: list[str] = []
    coordinator.register("test.handler", lambda t: calls.append(t.id))
    kairos = KairosCore(config, coordinator, memory)
    kairos.register_trigger(
        TimeTrigger(timedelta(seconds=0), "test.handler"),
    )
    kairos.start()
    time.sleep(0.3)
    kairos.stop()
    assert kairos.state.tick_count >= 2
    assert len(calls) >= 2


def test_no_threads_left_after_stop(config: KairosConfig, memory: MemoryManager,
                                    coordinator: Coordinator) -> None:
    kairos = KairosCore(config, coordinator, memory)
    kairos.start()
    time.sleep(0.15)
    kairos.stop()
    # Give daemon threads a moment to wind down.
    time.sleep(0.1)
    alive = [t for t in threading.enumerate() if "kairos" in t.name.lower()]
    assert alive == [], f"leftover threads: {alive}"


def test_inactivity_trigger_fires_after_silence(
    config: KairosConfig, memory: MemoryManager,
    coordinator: Coordinator, fake_clock: FakeClock,
) -> None:
    calls: list[str] = []
    coordinator.register("dream", lambda t: calls.append(t.id))
    kairos = KairosCore(config, coordinator, memory)
    kairos.register_trigger(
        InactivityTrigger(timedelta(seconds=2), "dream"),
    )
    # Manually drive ticks with the fake clock so we don't wait wall time.
    kairos.tick()  # t=0, no inactivity yet
    coordinator.run_pending()
    assert len(calls) == 0

    fake_clock.advance(3)
    kairos.tick()  # t=3, 3s silence → fire
    coordinator.run_pending()
    assert len(calls) == 1

    fake_clock.advance(1)
    kairos.tick()  # t=4, already fired for this window
    coordinator.run_pending()
    assert len(calls) == 1, "should not re-fire until new activity"

    # Simulate activity then silence again
    kairos.record_user_activity()
    fake_clock.advance(3)
    kairos.tick()
    coordinator.run_pending()
    assert len(calls) == 2, "should fire again after new silence"


def test_tick_direct_call_does_not_crash_without_start(
    config: KairosConfig, memory: MemoryManager, coordinator: Coordinator,
) -> None:
    kairos = KairosCore(config, coordinator, memory)
    kairos.tick()  # should work even though start() wasn't called
    assert kairos.state.tick_count == 1
