"""In-memory snapshot of what Kairos currently believes about itself.

Distinct from MemoryManager (which is durable) — this is the working set the
tick loop uses to decide what to do next, refreshed on demand from memory.
"""
from __future__ import annotations

from dataclasses import dataclass, field
from datetime import datetime
from typing import Any


@dataclass
class KairosState:
    started_at: datetime
    last_tick_at: datetime | None = None
    last_user_activity_at: datetime | None = None
    profile: dict[str, Any] = field(default_factory=dict)
    active_plan_id: int | None = None
    tick_count: int = 0
    context: dict[str, Any] = field(default_factory=dict)

    def mark_user_activity(self, when: datetime) -> None:
        self.last_user_activity_at = when

    def mark_tick(self, when: datetime) -> None:
        self.last_tick_at = when
        self.tick_count += 1
