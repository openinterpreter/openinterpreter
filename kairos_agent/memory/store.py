"""Storage backends: SQLite for events/plans, JSON file for the profile."""
from __future__ import annotations

import json
import sqlite3
import threading
from datetime import datetime
from pathlib import Path
from typing import Iterable

from .migrations import apply_schema
from .schemas import EventRecord, PlanRecord, ProfileRecord, TaskRecord


_ISO = "%Y-%m-%dT%H:%M:%S.%f"


def _iso(ts: datetime) -> str:
    return ts.strftime(_ISO)


def _parse_iso(value: str) -> datetime:
    return datetime.strptime(value, _ISO)


class SQLiteStore:
    """Thin wrapper over a SQLite connection.

    The connection is opened with `check_same_thread=False` and protected by
    a single re-entrant lock so it can safely be shared between Kairos's
    background threads (Coordinator worker, tick loop) without per-thread
    connection juggling. Volume is low — this trades a little contention for
    a much simpler ownership model.
    """

    def __init__(self, db_path: Path):
        self._db_path = db_path
        self._lock = threading.RLock()
        self._conn = sqlite3.connect(str(db_path), check_same_thread=False)
        self._conn.row_factory = sqlite3.Row
        apply_schema(self._conn)

    def close(self) -> None:
        with self._lock:
            self._conn.close()

    # ----- events -----------------------------------------------------------

    def insert_event(self, event: EventRecord) -> int:
        with self._lock:
            cur = self._conn.execute(
                "INSERT INTO events (ts, kind, payload, session_id, consolidated) "
                "VALUES (?, ?, ?, ?, ?)",
                (
                    _iso(event.ts),
                    event.kind,
                    json.dumps(event.payload, ensure_ascii=False),
                    event.session_id,
                    1 if event.consolidated else 0,
                ),
            )
            self._conn.commit()
            return int(cur.lastrowid)

    def select_events(
        self,
        since: datetime | None = None,
        consolidated: bool | None = None,
        limit: int = 100,
    ) -> list[EventRecord]:
        clauses: list[str] = []
        params: list[object] = []
        if since is not None:
            clauses.append("ts >= ?")
            params.append(_iso(since))
        if consolidated is not None:
            clauses.append("consolidated = ?")
            params.append(1 if consolidated else 0)
        where = f" WHERE {' AND '.join(clauses)}" if clauses else ""
        sql = f"SELECT * FROM events{where} ORDER BY ts ASC LIMIT ?"
        params.append(limit)
        with self._lock:
            rows = self._conn.execute(sql, params).fetchall()
        return [self._row_to_event(r) for r in rows]

    def mark_events_consolidated(self, ids: Iterable[int]) -> int:
        ids = list(ids)
        if not ids:
            return 0
        placeholders = ",".join("?" * len(ids))
        with self._lock:
            cur = self._conn.execute(
                f"UPDATE events SET consolidated = 1 WHERE id IN ({placeholders})",
                ids,
            )
            self._conn.commit()
            return cur.rowcount

    @staticmethod
    def _row_to_event(row: sqlite3.Row) -> EventRecord:
        return EventRecord(
            id=row["id"],
            ts=_parse_iso(row["ts"]),
            kind=row["kind"],
            payload=json.loads(row["payload"]),
            session_id=row["session_id"],
            consolidated=bool(row["consolidated"]),
        )

    # ----- plans ------------------------------------------------------------

    def insert_plan(self, plan: PlanRecord) -> int:
        with self._lock:
            cur = self._conn.execute(
                "INSERT INTO plans (goal, tasks, status, created_at) VALUES (?, ?, ?, ?)",
                (
                    plan.goal,
                    json.dumps([t.__dict__ for t in plan.tasks], ensure_ascii=False),
                    plan.status,
                    _iso(plan.created_at),
                ),
            )
            self._conn.commit()
            return int(cur.lastrowid)

    def get_plan(self, plan_id: int) -> PlanRecord | None:
        with self._lock:
            row = self._conn.execute(
                "SELECT * FROM plans WHERE id = ?", (plan_id,)
            ).fetchone()
        return self._row_to_plan(row) if row else None

    def update_plan_tasks(self, plan_id: int, tasks: list[TaskRecord]) -> None:
        with self._lock:
            self._conn.execute(
                "UPDATE plans SET tasks = ? WHERE id = ?",
                (json.dumps([t.__dict__ for t in tasks], ensure_ascii=False), plan_id),
            )
            self._conn.commit()

    def update_plan_status(self, plan_id: int, status: str) -> None:
        with self._lock:
            self._conn.execute(
                "UPDATE plans SET status = ? WHERE id = ?", (status, plan_id)
            )
            self._conn.commit()

    @staticmethod
    def _row_to_plan(row: sqlite3.Row) -> PlanRecord:
        raw_tasks = json.loads(row["tasks"])
        tasks = [TaskRecord(**t) for t in raw_tasks]
        return PlanRecord(
            id=row["id"],
            goal=row["goal"],
            tasks=tasks,
            status=row["status"],
            created_at=_parse_iso(row["created_at"]),
        )


class JSONProfileStore:
    """Single-file JSON store for the user profile.

    Lock-protected because Kairos may consolidate from a worker thread while
    the main thread reads the profile to enrich a prompt.
    """

    def __init__(self, profile_path: Path):
        self._path = profile_path
        self._lock = threading.RLock()

    def load(self) -> ProfileRecord:
        with self._lock:
            if not self._path.exists():
                return ProfileRecord()
            raw = json.loads(self._path.read_text(encoding="utf-8"))
        return ProfileRecord(
            user_id=raw.get("user_id", "default"),
            data=raw.get("data", {}),
            updated_at=_parse_iso(raw["updated_at"]) if "updated_at" in raw
            else datetime.utcnow(),
        )

    def save(self, profile: ProfileRecord) -> None:
        profile.updated_at = datetime.utcnow()
        payload = {
            "user_id": profile.user_id,
            "data": profile.data,
            "updated_at": _iso(profile.updated_at),
        }
        with self._lock:
            tmp = self._path.with_suffix(self._path.suffix + ".tmp")
            tmp.write_text(json.dumps(payload, ensure_ascii=False, indent=2),
                           encoding="utf-8")
            tmp.replace(self._path)
