"""Deterministic LLMClient implementation for tests and offline smoke runs.

Returns predictable strings without making any network calls. Two modes:
- Default: echoes a tagged version of the prompt, useful for assertions.
- Scripted: returns canned responses from a queue in order, falling back to
  echo when the queue is exhausted.
"""
from __future__ import annotations

from collections import deque
from typing import Iterable

from .interfaces import LLMClient


class DummyLLMClient(LLMClient):
    def __init__(self, scripted_responses: Iterable[str] | None = None):
        self._queue: deque[str] = deque(scripted_responses or [])
        self.call_count = 0
        self.last_prompt: str | None = None

    def complete(self, prompt: str, **kwargs) -> str:
        self.call_count += 1
        self.last_prompt = prompt
        if self._queue:
            return self._queue.popleft()
        # Echo mode: short, deterministic, parseable.
        return f"[dummy-llm] {prompt[:120]}"

    def push_response(self, response: str) -> None:
        self._queue.append(response)
