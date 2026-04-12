"""Summarizer used by AutoDream.

Wraps an LLMClient with a deterministic fallback so the consolidation pipeline
remains usable in tests and offline runs.
"""
from __future__ import annotations

import json

from ..common.interfaces import LLMClient
from ..memory.schemas import EventRecord


_PROMPT_TEMPLATE = (
    "You are an autonomous agent's memory consolidator. Read the following "
    "recent events and produce a concise (max 6 sentences) summary that "
    "captures the user's intent, decisions made, and anything worth remembering "
    "for future sessions. Respond with plain text only.\n\n"
    "Events (JSON):\n{events}\n\nSummary:"
)


class Summarizer:
    def __init__(self, llm: LLMClient | None):
        self._llm = llm

    def summarize(self, events: list[EventRecord]) -> str:
        if not events:
            return ""
        if self._llm is None:
            return self._fallback(events)
        prompt = _PROMPT_TEMPLATE.format(events=self._render_events(events))
        try:
            return self._llm.complete(prompt).strip()
        except Exception:  # noqa: BLE001 — never let consolidation crash
            return self._fallback(events)

    @staticmethod
    def _render_events(events: list[EventRecord]) -> str:
        rendered = [
            {
                "ts": e.ts.isoformat(),
                "kind": e.kind,
                "payload": e.payload,
            }
            for e in events
        ]
        return json.dumps(rendered, ensure_ascii=False, indent=2)

    @staticmethod
    def _fallback(events: list[EventRecord]) -> str:
        kinds: dict[str, int] = {}
        for e in events:
            kinds[e.kind] = kinds.get(e.kind, 0) + 1
        breakdown = ", ".join(f"{k}={v}" for k, v in sorted(kinds.items()))
        return f"[fallback summary] {len(events)} events: {breakdown}"
