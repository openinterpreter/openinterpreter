"""Public memory API used by the rest of Kairos.

MemoryManager is the *only* class outside the memory package that the rest of
Kairos imports. It hides the SQLite/JSON split behind plain method calls.
"""
from __future__ import annotations

from datetime import datetime
from typing import Any, Iterable

from ..common.logger import get_logger
from ..config import KairosConfig
from .schemas import EventRecord, PlanRecord, ProfileRecord, TaskRecord
from .store import JSONProfileStore, SQLiteStore


_log = get_logger("memory")


class MemoryManager:
    def __init__(self, config: KairosConfig):
        self._config = config
        storage = config.storage_path
        # storage_path is guaranteed to exist by KairosCore.start(); but if a
        # caller instantiates MemoryManager directly we make sure it does.
        storage.mkdir(parents=True, exist_ok=True)
        self._sqlite = SQLiteStore(storage / "kairos.db")
        self._profile = JSONProfileStore(storage / "profile.json")
        _log.info("memory.ready", extra={"db": str(storage / "kairos.db")})

    def close(self) -> None:
        self._sqlite.close()

    # ----- events -----------------------------------------------------------

    def save_event(
        self,
        kind: str,
        payload: dict[str, Any],
        session_id: str | None = None,
    ) -> int:
        event = EventRecord(
            id=None,
            ts=self._config.clock.now(),
            kind=kind,
            payload=payload,
            session_id=session_id,
        )
        new_id = self._sqlite.insert_event(event)
        _log.info("memory.event_saved", extra={"id": new_id, "kind": kind})
        return new_id

    def load_events(
        self,
        since: datetime | None = None,
        consolidated: bool | None = None,
        limit: int = 100,
    ) -> list[EventRecord]:
        return self._sqlite.select_events(since=since, consolidated=consolidated,
                                          limit=limit)

    def mark_events_consolidated(self, ids: Iterable[int]) -> int:
        n = self._sqlite.mark_events_consolidated(ids)
        _log.info("memory.events_consolidated", extra={"count": n})
        return n

    # ----- profile ----------------------------------------------------------

    def load_profile(self) -> ProfileRecord:
        return self._profile.load()

    def save_profile(self, profile: ProfileRecord) -> None:
        self._profile.save(profile)
        _log.info("memory.profile_saved", extra={"user_id": profile.user_id})

    def update_profile(self, **patch: Any) -> ProfileRecord:
        """Convenience: load, merge top-level keys into `data`, save."""
        profile = self.load_profile()
        profile.data.update(patch)
        self.save_profile(profile)
        return profile

    # ----- plans ------------------------------------------------------------

    def save_plan(self, plan: PlanRecord) -> int:
        new_id = self._sqlite.insert_plan(plan)
        plan.id = new_id
        _log.info("memory.plan_saved", extra={"id": new_id, "goal": plan.goal[:60]})
        return new_id

    def get_plan(self, plan_id: int) -> PlanRecord | None:
        return self._sqlite.get_plan(plan_id)

    def update_plan_task(
        self,
        plan_id: int,
        task_id: str,
        patch: dict[str, Any],
    ) -> PlanRecord:
        plan = self._sqlite.get_plan(plan_id)
        if plan is None:
            raise KeyError(f"plan {plan_id} not found")
        for task in plan.tasks:
            if task.id == task_id:
                for key, value in patch.items():
                    if not hasattr(task, key):
                        raise AttributeError(f"unknown task field: {key}")
                    setattr(task, key, value)
                break
        else:
            raise KeyError(f"task {task_id} not found in plan {plan_id}")
        self._sqlite.update_plan_tasks(plan_id, plan.tasks)
        return plan

    def set_plan_status(self, plan_id: int, status: str) -> None:
        self._sqlite.update_plan_status(plan_id, status)


def task_record(
    id: str,
    description: str,
    deps: list[str] | None = None,
    priority: int = 5,
    status: str = "pending",
) -> TaskRecord:
    """Small helper so callers don't need to import schemas just to build tasks."""
    return TaskRecord(
        id=id,
        description=description,
        deps=deps or [],
        priority=priority,
        status=status,
    )
