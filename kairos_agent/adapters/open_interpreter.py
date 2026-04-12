"""Adapter that bridges open-interpreter ↔ kairos_agent.

Zero modifications to interpreter/core/core.py — everything is wired via
composition and monkey-patching of the interpreter instance at runtime.

Usage
-----
    from interpreter import interpreter
    from kairos_agent.adapters.open_interpreter import attach_to_interpreter

    kairos = attach_to_interpreter(interpreter, tick_interval=10.0)
    interpreter.chat("Hello!")
    # ... Kairos observes, consolidates, plans in background ...
    kairos.stop()
"""
from __future__ import annotations

import functools
import logging
from datetime import timedelta
from pathlib import Path
from typing import TYPE_CHECKING, Any

from ..autodream import AutoDream
from ..common.interfaces import LLMClient
from ..common.logger import get_logger
from ..config import KairosConfig
from ..coordinator import Coordinator
from ..kairos import InactivityTrigger, KairosCore
from ..memory import MemoryManager
from ..ultraplan import UltraPlan

if TYPE_CHECKING:
    from interpreter.core.core import OpenInterpreter

_log = get_logger("adapter.oi")


# ---------------------------------------------------------------------------
# 1. LLMClient adapter
# ---------------------------------------------------------------------------

class OpenInterpreterLLMClient(LLMClient):
    """Adapts ``interpreter.llm.run()`` to the ``LLMClient.complete()`` ABC.

    ``interpreter.llm.run(messages)`` is a **generator** that yields LMC chunks
    (``{"type": "message", "content": "..."}``).  We iterate the generator,
    concatenate all message-type chunks, and return a single string.
    """

    def __init__(self, interpreter: "OpenInterpreter"):
        self._interpreter = interpreter

    def complete(self, prompt: str, **kwargs: Any) -> str:
        system_text = kwargs.pop("system", "You are a helpful assistant.")

        # Build LMC-format messages expected by interpreter.llm.run().
        messages = [
            {"role": "system", "type": "message", "content": system_text},
            {"role": "user", "type": "message", "content": prompt},
        ]

        parts: list[str] = []
        try:
            for chunk in self._interpreter.llm.run(messages):
                if chunk.get("type") == "message":
                    content = chunk.get("content", "")
                    if content:
                        parts.append(content)
        except Exception as exc:
            _log.exception("adapter.llm_error", extra={"err": str(exc)})
            raise

        return "".join(parts)


# ---------------------------------------------------------------------------
# 2. Chat wrapper for automatic event recording
# ---------------------------------------------------------------------------

def _wrap_chat(original_chat, kairos: KairosCore, memory: MemoryManager):
    """Return a wrapper around ``interpreter.chat()`` that records events.

    Before the call we signal user activity (re-arms InactivityTrigger).
    After a successful call we persist the new messages as memory events so
    AutoDream can consolidate them later.
    """

    @functools.wraps(original_chat)
    def wrapped(message=None, display=True, stream=False, blocking=True):
        # Record user activity (text) for the inactivity trigger.
        if message is not None:
            text = message if isinstance(message, str) else str(message)[:200]
            kairos.record_user_activity({"text": text})

        # Delegate to the real chat().
        result = original_chat(message=message, display=display,
                               stream=stream, blocking=blocking)

        # After the chat round-trip, persist new assistant messages.
        if blocking and not stream:
            try:
                interpreter = original_chat.__self__  # type: ignore[attr-defined]
                new_msgs = interpreter.messages[interpreter.last_messages_count:]
                for msg in new_msgs:
                    memory.save_event(
                        kind=f"{msg.get('role', 'unknown')}_{msg.get('type', 'unknown')}",
                        payload={
                            "content": str(msg.get("content", ""))[:500],
                            "format": msg.get("format"),
                        },
                    )
            except Exception:
                _log.exception("adapter.event_save_error")

        return result

    return wrapped


# ---------------------------------------------------------------------------
# 3. Helper: one-call wiring
# ---------------------------------------------------------------------------

def attach_to_interpreter(
    interpreter: "OpenInterpreter",
    *,
    storage_path: str | Path | None = None,
    tick_interval: float = 30.0,
    inactivity_seconds: float = 300.0,
    log_level: int = logging.INFO,
    log_to_console: bool = False,
) -> KairosCore:
    """Wire Kairos into an existing ``OpenInterpreter`` instance.

    Creates the full module graph (config, memory, coordinator, autodream,
    ultraplan, kairos) and starts the background tick loop. The interpreter's
    ``chat()`` method is transparently wrapped so every conversation is
    recorded as memory events — no manual calls required.

    Parameters
    ----------
    interpreter
        A live ``OpenInterpreter`` instance (``from interpreter import interpreter``).
    storage_path
        Where to store kairos.db, profile.json, and logs. Defaults to
        ``~/.kairos_agent``.
    tick_interval
        Seconds between Kairos ticks.
    inactivity_seconds
        Seconds of silence before AutoDream is triggered.
    log_level / log_to_console
        Forwarded to ``configure_logging``.

    Returns
    -------
    KairosCore
        The started agent. Call ``.stop()`` when done.
    """
    # Resolve storage path — prefer the host's storage dir if available.
    if storage_path is None:
        try:
            from interpreter.terminal_interface.utils.local_storage_path import (
                get_storage_path,
            )
            storage_path = get_storage_path("kairos")
        except ImportError:
            storage_path = None  # falls back to ~/.kairos_agent

    # --- Config ---
    llm = OpenInterpreterLLMClient(interpreter)
    config = KairosConfig(
        storage_path=storage_path,
        tick_interval=tick_interval,
        log_level=log_level,
        log_to_console=log_to_console,
        llm_client=llm,
    )

    # --- Memory ---
    memory = MemoryManager(config)

    # --- Coordinator + handlers ---
    coordinator = Coordinator()
    autodream = AutoDream(config, memory)
    ultraplan = UltraPlan(llm, memory)
    coordinator.register("autodream.run", autodream.run)
    coordinator.register("ultraplan.generate",
                         lambda task: ultraplan.generate(task.payload.get("goal", "")))

    # --- KairosCore + triggers ---
    kairos = KairosCore(config, coordinator, memory)
    kairos.register_trigger(
        InactivityTrigger(
            silence=timedelta(seconds=inactivity_seconds),
            handler_name="autodream.run",
            priority=3,
        ),
    )

    # --- Wrap chat ---
    interpreter.chat = _wrap_chat(interpreter.chat, kairos, memory)

    # --- Attach and start ---
    interpreter.kairos = kairos  # type: ignore[attr-defined]
    interpreter.kairos_memory = memory  # type: ignore[attr-defined]
    kairos.start()
    _log.info("adapter.attached", extra={
        "model": getattr(interpreter.llm, "model", "unknown"),
        "tick_interval": tick_interval,
        "inactivity_seconds": inactivity_seconds,
    })

    return kairos


def detach_from_interpreter(interpreter: "OpenInterpreter") -> None:
    """Stop Kairos and restore the original ``chat()`` method."""
    kairos = getattr(interpreter, "kairos", None)
    if kairos is not None:
        kairos.stop()

    # Restore original chat if it was wrapped.
    if hasattr(interpreter.chat, "__wrapped__"):
        interpreter.chat = interpreter.chat.__wrapped__

    # Clean up the attributes we added.
    for attr in ("kairos", "kairos_memory"):
        if hasattr(interpreter, attr):
            delattr(interpreter, attr)

    _log.info("adapter.detached")
