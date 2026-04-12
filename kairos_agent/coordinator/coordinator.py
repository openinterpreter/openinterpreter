"""Internal task router.

The Coordinator is the single writer to the persistent modules. Kairos and
external code never call AutoDream/UltraPlan directly — they enqueue a
TaskRequest with a `handler_name` and the Coordinator dispatches it on its
worker thread, holding a per-module lock to prevent conflicts.
"""
from __future__ import annotations

import queue
import threading
from typing import Callable

from ..common.dto import TaskRequest
from ..common.logger import get_logger
from .locks import LockRegistry


_log = get_logger("coordinator")

#: A handler is any callable that takes a TaskRequest. Return value is logged
#: but otherwise ignored — handlers are expected to mutate state via MemoryManager.
Handler = Callable[[TaskRequest], object]


class Coordinator:
    def __init__(self) -> None:
        self._queue: queue.PriorityQueue[tuple[int, TaskRequest]] = queue.PriorityQueue()
        self._registry: dict[str, Handler] = {}
        self._locks = LockRegistry()
        self._stop_event = threading.Event()
        self._worker: threading.Thread | None = None

    # ----- registration -----------------------------------------------------

    def register(self, handler_name: str, handler: Handler) -> None:
        if handler_name in self._registry:
            raise ValueError(f"handler already registered: {handler_name}")
        self._registry[handler_name] = handler
        _log.info("coordinator.handler_registered", extra={"handler": handler_name})

    # ----- enqueue / dispatch ----------------------------------------------

    def enqueue(self, task: TaskRequest) -> None:
        _log.info("coordinator.enqueue", extra={
            "id": task.id,
            "handler": task.handler_name,
            "priority": task.priority,
            "source": task.source,
        })
        self._queue.put((task.priority, task))

    def _dispatch(self, task: TaskRequest) -> None:
        handler = self._registry.get(task.handler_name)
        if handler is None:
            _log.warning("coordinator.no_handler", extra={
                "id": task.id, "handler": task.handler_name,
            })
            return
        # Lock by module prefix: "autodream.run" -> "autodream".
        module_name = task.handler_name.split(".", 1)[0]
        with self._locks.acquire(module_name) as acquired:
            if not acquired:
                _log.warning("coordinator.lock_unavailable", extra={
                    "id": task.id, "module": module_name,
                })
                return
            try:
                handler(task)
                _log.info("coordinator.task_done", extra={
                    "id": task.id, "handler": task.handler_name,
                })
            except Exception as exc:  # noqa: BLE001 — log and continue
                _log.exception("coordinator.task_error", extra={
                    "id": task.id, "handler": task.handler_name, "err": str(exc),
                })

    # ----- worker thread ---------------------------------------------------

    def start(self) -> None:
        if self._worker is not None and self._worker.is_alive():
            return
        self._stop_event.clear()
        self._worker = threading.Thread(
            target=self._run, name="kairos-coordinator", daemon=True,
        )
        self._worker.start()
        _log.info("coordinator.started")

    def stop(self, timeout: float = 5.0) -> None:
        self._stop_event.set()
        # Wake the worker so it can observe the stop flag.
        # We push a sentinel with the highest possible priority.
        self._queue.put((-1, _SENTINEL))
        if self._worker is not None:
            self._worker.join(timeout=timeout)
        _log.info("coordinator.stopped")

    def run_pending(self) -> int:
        """Drain the queue synchronously (used by tests). Returns task count."""
        count = 0
        while not self._queue.empty():
            _, task = self._queue.get()
            if task is _SENTINEL:
                continue
            self._dispatch(task)
            count += 1
        return count

    def _run(self) -> None:
        while not self._stop_event.is_set():
            try:
                _, task = self._queue.get(timeout=0.1)
            except queue.Empty:
                continue
            if task is _SENTINEL:
                continue
            self._dispatch(task)


# Sentinel sortable by PriorityQueue. Using a TaskRequest with a fixed id keeps
# tuple comparison happy.
_SENTINEL = TaskRequest(handler_name="__sentinel__", priority=-1)
