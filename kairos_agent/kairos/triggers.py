"""Trigger primitives evaluated each tick.

A Trigger looks at KairosState and returns either None (do nothing) or a
TaskRequest to enqueue. Triggers are stateless aside from the small bookkeeping
they need to detect "fired since last tick".
"""
from __future__ import annotations

from abc import ABC, abstractmethod
from datetime import timedelta

from ..common.dto import TaskRequest
from .state import KairosState


class Trigger(ABC):
    """Pure decision unit. Inspects state, optionally produces a TaskRequest."""

    @abstractmethod
    def evaluate(self, state: KairosState) -> TaskRequest | None:
        ...


class TimeTrigger(Trigger):
    """Fires once every `interval`, regardless of activity."""

    def __init__(self, interval: timedelta, handler_name: str,
                 priority: int = 5, payload: dict | None = None):
        self._interval = interval
        self._handler = handler_name
        self._priority = priority
        self._payload = payload or {}
        self._last_fired_at = None  # type: ignore[var-annotated]

    def evaluate(self, state: KairosState) -> TaskRequest | None:
        now = state.last_tick_at
        if now is None:
            return None
        if self._last_fired_at is None or (now - self._last_fired_at) >= self._interval:
            self._last_fired_at = now
            return TaskRequest(
                handler_name=self._handler,
                priority=self._priority,
                payload=dict(self._payload),
                source="kairos.time_trigger",
            )
        return None


class InactivityTrigger(Trigger):
    """Fires after `silence` seconds without observed user activity.

    Re-arms only after another user activity is observed, so it doesn't
    re-fire on every tick once the threshold is past.
    """

    def __init__(self, silence: timedelta, handler_name: str,
                 priority: int = 5, payload: dict | None = None):
        self._silence = silence
        self._handler = handler_name
        self._priority = priority
        self._payload = payload or {}
        self._fired_for_window: bool = False
        self._last_seen_activity = None  # type: ignore[var-annotated]

    def evaluate(self, state: KairosState) -> TaskRequest | None:
        now = state.last_tick_at
        if now is None:
            return None
        if state.last_user_activity_at != self._last_seen_activity:
            # New activity since we last looked → reset the latch.
            self._last_seen_activity = state.last_user_activity_at
            self._fired_for_window = False
        if self._fired_for_window:
            return None
        # If we have no activity baseline, use Kairos start time so the trigger
        # can still fire after long idle uptime.
        baseline = state.last_user_activity_at or state.started_at
        if (now - baseline) >= self._silence:
            self._fired_for_window = True
            return TaskRequest(
                handler_name=self._handler,
                priority=self._priority,
                payload=dict(self._payload),
                source="kairos.inactivity_trigger",
            )
        return None
