"""Background tick driver.

Tiny on purpose: it's a daemon thread that calls a callback every `interval`
seconds until told to stop. Kairos owns the callback and the state mutation;
the scheduler is just the timing mechanism, isolated so it can be swapped for
asyncio or APScheduler later without touching KairosCore.
"""
from __future__ import annotations

import threading
from typing import Callable


class Scheduler:
    def __init__(self, interval: float, on_tick: Callable[[], None],
                 name: str = "kairos-scheduler"):
        self._interval = interval
        self._on_tick = on_tick
        self._stop = threading.Event()
        self._thread: threading.Thread | None = None
        self._name = name

    @property
    def is_running(self) -> bool:
        return self._thread is not None and self._thread.is_alive()

    def start(self) -> None:
        if self.is_running:
            return
        self._stop.clear()
        self._thread = threading.Thread(target=self._run, name=self._name, daemon=True)
        self._thread.start()

    def stop(self, timeout: float = 5.0) -> None:
        self._stop.set()
        if self._thread is not None:
            self._thread.join(timeout=timeout)

    def _run(self) -> None:
        # First tick fires immediately so callers don't wait a full interval
        # to see Kairos do anything on startup.
        while not self._stop.is_set():
            try:
                self._on_tick()
            except Exception:  # noqa: BLE001 — never let the loop die
                # Logged inside the callback; we just don't propagate.
                pass
            # Use Event.wait so stop() can interrupt the sleep immediately.
            if self._stop.wait(self._interval):
                break
