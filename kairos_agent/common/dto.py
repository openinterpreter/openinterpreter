"""Internal DTOs passed between Kairos modules.

These are deliberately *not* the persistence schemas (which live in
memory/schemas.py). Keeping them separate lets the persistence layer evolve
without breaking the in-memory message bus.
"""
from __future__ import annotations

import uuid
from dataclasses import dataclass, field
from datetime import datetime
from typing import Any


@dataclass(frozen=True)
class TriggerEvent:
    """Emitted by a Trigger when it decides Kairos should act."""
    name: str
    fired_at: datetime
    payload: dict[str, Any] = field(default_factory=dict)


@dataclass
class TaskRequest:
    """A unit of work the Coordinator will dispatch to a registered handler.

    `handler_name` matches a key in Coordinator's registry, e.g.
    "autodream.run" or "ultraplan.generate". `payload` carries handler args.
    """
    handler_name: str
    payload: dict[str, Any] = field(default_factory=dict)
    priority: int = 5  # 1 = urgent, 10 = optional
    id: str = field(default_factory=lambda: uuid.uuid4().hex)
    created_at: datetime = field(default_factory=datetime.utcnow)
    source: str = "kairos"  # who enqueued it (for logs)

    def __lt__(self, other: "TaskRequest") -> bool:
        # Required so PriorityQueue can break priority ties without crashing.
        if not isinstance(other, TaskRequest):
            return NotImplemented
        return self.created_at < other.created_at
