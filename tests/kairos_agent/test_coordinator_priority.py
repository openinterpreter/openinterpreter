"""Coordinator: priority ordering + lock serialization."""
from __future__ import annotations

import threading
import time

from kairos_agent import Coordinator, TaskRequest


def test_priority_order(coordinator: Coordinator) -> None:
    calls: list[str] = []
    coordinator.register("test.do", lambda task: calls.append(task.id))
    coordinator.enqueue(TaskRequest(handler_name="test.do", priority=10, id="c"))
    coordinator.enqueue(TaskRequest(handler_name="test.do", priority=1, id="a"))
    coordinator.enqueue(TaskRequest(handler_name="test.do", priority=5, id="b"))
    coordinator.run_pending()
    assert calls == ["a", "b", "c"]


def test_worker_thread_dispatches(coordinator: Coordinator) -> None:
    calls: list[str] = []
    coordinator.register("test.do", lambda task: calls.append(task.id))
    coordinator.start()
    coordinator.enqueue(TaskRequest(handler_name="test.do", id="w1"))
    coordinator.enqueue(TaskRequest(handler_name="test.do", id="w2"))
    time.sleep(0.3)
    coordinator.stop()
    assert "w1" in calls
    assert "w2" in calls


def test_unknown_handler_does_not_crash(coordinator: Coordinator) -> None:
    coordinator.enqueue(TaskRequest(handler_name="nonexistent", id="x"))
    coordinator.run_pending()  # should log a warning, not raise


def test_lock_serialization(coordinator: Coordinator) -> None:
    """Two tasks targeting the same module should not run concurrently."""
    running = threading.Event()
    overlap_detected = []

    def slow_handler(task: TaskRequest) -> None:
        if running.is_set():
            overlap_detected.append(True)
        running.set()
        time.sleep(0.15)
        running.clear()

    coordinator.register("mod.action", slow_handler)
    coordinator.start()
    coordinator.enqueue(TaskRequest(handler_name="mod.action", priority=1, id="s1"))
    coordinator.enqueue(TaskRequest(handler_name="mod.action", priority=1, id="s2"))
    time.sleep(0.5)
    coordinator.stop()
    assert not overlap_detected, "handlers ran concurrently — lock failed"
