"""KairosCore — the persistent agent loop.

Owns the Scheduler, the KairosState, and the list of Triggers. On every tick
it (1) refreshes state from MemoryManager, (2) asks each Trigger if it wants
to fire, and (3) enqueues any produced TaskRequests on the Coordinator. Kairos
itself is intentionally dumb: all the actual work happens in handlers
registered on the Coordinator.
"""
from __future__ import annotations

from typing import Any

from ..common.dto import TaskRequest
from ..common.logger import configure_logging, get_logger
from ..common.paths import ensure_storage_dir
from ..config import KairosConfig
from ..coordinator.coordinator import Coordinator
from ..memory.manager import MemoryManager
from .scheduler import Scheduler
from .state import KairosState
from .triggers import Trigger


_log = get_logger("kairos")


class KairosCore:
    def __init__(
        self,
        config: KairosConfig,
        coordinator: Coordinator,
        memory: MemoryManager,
    ) -> None:
        self._config = config
        self._coordinator = coordinator
        self._memory = memory
        self._triggers: list[Trigger] = []
        self._state = KairosState(started_at=config.clock.now())
        self._scheduler = Scheduler(
            interval=config.tick_interval,
            on_tick=self.tick,
            name="kairos-tick",
        )

    # ----- public API -------------------------------------------------------

    @property
    def state(self) -> KairosState:
        return self._state

    def register_trigger(self, trigger: Trigger) -> None:
        self._triggers.append(trigger)
        _log.info("kairos.trigger_registered",
                  extra={"type": type(trigger).__name__})

    def record_user_activity(self, payload: dict[str, Any] | None = None) -> None:
        """Tell Kairos the user just did something.

        Persists an event to memory and updates state so InactivityTrigger
        re-arms. Hosts call this from their input loop (e.g. open-interpreter
        adapter calls it from `_streaming_chat`).
        """
        now = self._config.clock.now()
        self._state.mark_user_activity(now)
        self._memory.save_event(
            kind="user_activity",
            payload=payload or {},
        )

    def start(self) -> None:
        ensure_storage_dir(self._config.storage_path)
        configure_logging(self._config.storage_path,
                          level=self._config.log_level,
                          console=self._config.log_to_console)
        # Refresh state from memory before starting the loop so triggers see
        # any prior profile.
        try:
            self._state.profile = self._memory.load_profile().data
        except Exception:  # noqa: BLE001 — fresh install path
            self._state.profile = {}
        self._coordinator.start()
        self._scheduler.start()
        _log.info("kairos.started", extra={
            "tick_interval": self._config.tick_interval,
            "trigger_count": len(self._triggers),
        })

    def stop(self) -> None:
        self._scheduler.stop()
        self._coordinator.stop()
        _log.info("kairos.stopped", extra={"ticks": self._state.tick_count})

    def tick(self) -> None:
        """One iteration of the loop. Public so tests can drive it directly."""
        now = self._config.clock.now()
        self._state.mark_tick(now)
        _log.info("kairos.tick", extra={"n": self._state.tick_count})
        produced: list[TaskRequest] = []
        for trigger in self._triggers:
            try:
                task = trigger.evaluate(self._state)
            except Exception as exc:  # noqa: BLE001
                _log.exception("kairos.trigger_error", extra={
                    "trigger": type(trigger).__name__, "err": str(exc),
                })
                continue
            if task is not None:
                produced.append(task)
        for task in produced:
            self._coordinator.enqueue(task)
