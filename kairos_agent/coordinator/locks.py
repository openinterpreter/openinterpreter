"""Named locks to prevent module-level conflicts.

Used by Coordinator to ensure that two TaskRequests targeting the same module
(e.g. autodream.run while another autodream.run is in flight) serialize.
"""
from __future__ import annotations

import threading
from contextlib import contextmanager
from typing import Iterator


class LockRegistry:
    def __init__(self) -> None:
        self._locks: dict[str, threading.Lock] = {}
        self._meta_lock = threading.Lock()

    def _get(self, name: str) -> threading.Lock:
        with self._meta_lock:
            lock = self._locks.get(name)
            if lock is None:
                lock = threading.Lock()
                self._locks[name] = lock
            return lock

    @contextmanager
    def acquire(self, name: str, timeout: float | None = None) -> Iterator[bool]:
        """Acquire the named lock for the duration of the with-block.

        Yields True if the lock was acquired, False on timeout. Always releases.
        """
        lock = self._get(name)
        acquired = lock.acquire(timeout=timeout) if timeout is not None else lock.acquire()
        try:
            yield acquired
        finally:
            if acquired:
                lock.release()
