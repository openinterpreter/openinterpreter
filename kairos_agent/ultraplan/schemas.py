"""Plan / Task dataclasses (in-memory representation).

Round-trip JSON via to_dict / from_dict — used both for LLM I/O and for
persistence (the memory store keeps tasks as a JSON column).
"""
from __future__ import annotations

from dataclasses import dataclass, field
from datetime import datetime
from typing import Any


@dataclass
class Task:
    id: str
    description: str
    deps: list[str] = field(default_factory=list)
    priority: int = 5  # 1 = urgent, 10 = optional
    status: str = "pending"  # pending | in_progress | done | blocked

    def to_dict(self) -> dict[str, Any]:
        return {
            "id": self.id,
            "description": self.description,
            "deps": list(self.deps),
            "priority": self.priority,
            "status": self.status,
        }

    @classmethod
    def from_dict(cls, raw: dict[str, Any]) -> "Task":
        return cls(
            id=str(raw["id"]),
            description=str(raw["description"]),
            deps=list(raw.get("deps", [])),
            priority=int(raw.get("priority", 5)),
            status=str(raw.get("status", "pending")),
        )


@dataclass
class Plan:
    goal: str
    tasks: list[Task]
    id: int | None = None
    status: str = "active"
    created_at: datetime = field(default_factory=datetime.utcnow)

    def to_dict(self) -> dict[str, Any]:
        return {
            "id": self.id,
            "goal": self.goal,
            "status": self.status,
            "created_at": self.created_at.isoformat(),
            "tasks": [t.to_dict() for t in self.tasks],
        }

    @classmethod
    def from_dict(cls, raw: dict[str, Any]) -> "Plan":
        created_at_raw = raw.get("created_at")
        if created_at_raw:
            created_at = datetime.fromisoformat(created_at_raw)
        else:
            created_at = datetime.utcnow()
        return cls(
            id=raw.get("id"),
            goal=str(raw["goal"]),
            status=str(raw.get("status", "active")),
            created_at=created_at,
            tasks=[Task.from_dict(t) for t in raw.get("tasks", [])],
        )
