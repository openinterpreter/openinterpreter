"""AutoDream — periodic memory consolidation.

Triggered by Kairos (typically after a period of user inactivity), AutoDream
reads unconsolidated events from MemoryManager, asks a Summarizer to compress
them, writes the result onto the user profile, and marks the source events
consolidated so they aren't summarized again.
"""
from __future__ import annotations

from typing import Any

from ..common.dto import TaskRequest
from ..common.logger import get_logger
from ..config import KairosConfig
from ..memory.manager import MemoryManager
from .summarizer import Summarizer


_log = get_logger("autodream")


class AutoDream:
    def __init__(self, config: KairosConfig, memory: MemoryManager,
                 summarizer: Summarizer | None = None):
        self._config = config
        self._memory = memory
        self._summarizer = summarizer or Summarizer(config.llm_client)

    def run(self, task: TaskRequest | None = None) -> dict[str, Any]:
        """Pipeline entry point. Returns a small report dict for logging/tests."""
        events = self._memory.load_events(consolidated=False, limit=500)
        _log.info("autodream.start", extra={"event_count": len(events)})
        if not events:
            return {"events_consolidated": 0, "summary": ""}

        summary = self._summarizer.summarize(events)
        profile = self._memory.load_profile()
        # Keep a rolling list of summaries so the agent has a memory of memories.
        history = profile.data.setdefault("summary_history", [])
        history.append({
            "ts": self._config.clock.now().isoformat(),
            "event_count": len(events),
            "summary": summary,
        })
        profile.data["last_summary"] = summary
        profile.data["last_consolidation"] = self._config.clock.now().isoformat()
        self._memory.save_profile(profile)

        ids = [e.id for e in events if e.id is not None]
        self._memory.mark_events_consolidated(ids)
        _log.info("autodream.done", extra={
            "events_consolidated": len(ids),
            "summary_chars": len(summary),
        })
        return {"events_consolidated": len(ids), "summary": summary}
