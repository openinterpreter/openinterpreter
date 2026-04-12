"""Abstract interfaces that decouple kairos_agent from any host application.

Every external dependency Kairos has on the outside world (LLM provider, wall
clock, event source) goes through one of these ABCs. This is what makes the
package standalone-testable: the host wires real implementations in, and tests
inject deterministic fakes.
"""
from __future__ import annotations

from abc import ABC, abstractmethod
from datetime import datetime
from typing import Callable


class LLMClient(ABC):
    """Minimal LLM contract Kairos depends on.

    Implementations adapt this to whatever provider the host uses
    (open-interpreter's Llm, raw OpenAI/Anthropic SDK, a local model, etc.).
    """

    @abstractmethod
    def complete(self, prompt: str, **kwargs) -> str:
        """Return a single completion string for the given prompt.

        kwargs are passed through to the underlying provider (model, temperature,
        max_tokens, ...). Implementations should silently ignore unknown kwargs.
        """


class Clock(ABC):
    """Wall-clock abstraction. Mockable in tests via FakeClock."""

    @abstractmethod
    def now(self) -> datetime:
        ...


class SystemClock(Clock):
    """Default real-time clock."""

    def now(self) -> datetime:
        return datetime.utcnow()


class EventBus(ABC):
    """Optional pub/sub for external triggers (e.g. host signals end of session).

    Kairos works without an EventBus — triggers fall back to time-based polling.
    Hosts that want push-based triggers implement this and pass it to KairosCore.
    """

    @abstractmethod
    def publish(self, topic: str, payload: dict) -> None:
        ...

    @abstractmethod
    def subscribe(self, topic: str, handler: Callable[[dict], None]) -> None:
        ...
