"""Persistence-layer dataclasses for memory.

These are returned to callers of MemoryManager. They are *not* the wire format
between Kairos modules — those live in kairos_agent/common/dto.py.
"""
from __future__ import annotations

from dataclasses import dataclass, field
from datetime import datetime
from typing import Any


@dataclass
class EventRecord:
    """A single observation Kairos has stored: an interaction, a decision, etc."""
    id: int | None              # None until the row has been inserted
    ts: datetime
    kind: str                   # free-form tag, e.g. "user_message", "tool_call"
    payload: dict[str, Any]
    session_id: str | None = None
    consolidated: bool = False  # set to True once AutoDream has summarized it


@dataclass
class ProfileRecord:
    """The agent's persistent picture of the user.

    Stored as a single JSON file (one user per Kairos instance for the MVP);
    `data` is fully user-defined so AutoDream and the host can extend it freely.
    """
    user_id: str = "default"
    data: dict[str, Any] = field(default_factory=dict)
    updated_at: datetime = field(default_factory=datetime.utcnow)


@dataclass
class TaskRecord:
    """One step inside a Plan. Mirror of ultraplan.schemas.Task for storage."""
    id: str
    description: str
    deps: list[str] = field(default_factory=list)
    priority: int = 5
    status: str = "pending"  # pending | in_progress | done | blocked


@dataclass
class PlanRecord:
    """A goal-decomposed plan persisted in SQLite."""
    id: int | None
    goal: str
    tasks: list[TaskRecord]
    status: str = "active"  # active | done | abandoned
    created_at: datetime = field(default_factory=datetime.utcnow)
