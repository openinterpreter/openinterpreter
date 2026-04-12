"""Tests for the open-interpreter adapter.

We *don't* import from `interpreter` here (it pulls in litellm, torch, etc.).
Instead we build a minimal mock that looks enough like OpenInterpreter for the
adapter to wire itself up.  This keeps the tests lightweight and CI-friendly.
"""
from __future__ import annotations

import tempfile
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import MagicMock

import pytest

from kairos_agent.adapters.open_interpreter import (
    OpenInterpreterLLMClient,
    attach_to_interpreter,
    detach_from_interpreter,
)
from kairos_agent.common import configure_logging, ensure_storage_dir


# ---------------------------------------------------------------------------
# Helpers / mocks
# ---------------------------------------------------------------------------

def _make_fake_interpreter(tmp_path: Path):
    """Build a minimal stand-in for OpenInterpreter.

    Just enough surface area for the adapter to latch on:
    - .llm with .run(messages) and .model
    - .chat(message, display, stream, blocking)
    - .messages, .last_messages_count
    """
    # Fake llm that yields two message chunks
    def fake_llm_run(messages):
        yield {"type": "message", "content": "Hello "}
        yield {"type": "message", "content": "world"}

    fake_llm = SimpleNamespace(
        run=fake_llm_run,
        model="fake-model",
    )

    # Fake chat — just appends messages to .messages
    interp = MagicMock()
    interp.llm = fake_llm
    interp.messages = []
    interp.last_messages_count = 0

    original_chat_calls: list[dict] = []

    def fake_chat(message=None, display=True, stream=False, blocking=True):
        original_chat_calls.append({"message": message})
        if message is not None:
            interp.messages.append(
                {"role": "user", "type": "message", "content": message}
            )
        interp.messages.append(
            {"role": "assistant", "type": "message", "content": "fake reply"}
        )
        interp.last_messages_count = len(interp.messages) - 1
        return interp.messages[interp.last_messages_count:]

    # Bind fake_chat properly so __self__ is available to the wrapper.
    interp.chat = fake_chat
    interp.chat.__self__ = interp  # the wrapper reads this
    interp._original_chat_calls = original_chat_calls

    return interp


# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------

class TestOpenInterpreterLLMClient:
    def test_complete_concatenates_message_chunks(self, tmp_path: Path) -> None:
        interp = _make_fake_interpreter(tmp_path)
        llm = OpenInterpreterLLMClient(interp)
        result = llm.complete("Say hi")
        assert result == "Hello world"

    def test_complete_ignores_code_chunks(self, tmp_path: Path) -> None:
        def run_with_code(messages):
            yield {"type": "message", "content": "Sure: "}
            yield {"type": "code", "format": "python", "content": "print(1)"}
            yield {"type": "message", "content": "done"}

        interp = _make_fake_interpreter(tmp_path)
        interp.llm.run = run_with_code
        llm = OpenInterpreterLLMClient(interp)
        assert llm.complete("run code") == "Sure: done"


class TestAttachDetach:
    def test_attach_starts_kairos_and_wraps_chat(self, tmp_path: Path) -> None:
        storage = ensure_storage_dir(tmp_path / "kairos")
        configure_logging(storage)
        interp = _make_fake_interpreter(tmp_path)

        kairos = attach_to_interpreter(
            interp,
            storage_path=storage,
            tick_interval=0.05,
            inactivity_seconds=999,
        )

        # Kairos is running
        assert hasattr(interp, "kairos")
        assert interp.kairos is kairos

        # Chat wrapper is in place — original chat still gets called
        interp.chat(message="test message")
        assert len(interp._original_chat_calls) == 1

        # Events were recorded in memory
        events = interp.kairos_memory.load_events()
        # At least the user_activity event from record_user_activity
        assert len(events) >= 1

        kairos.stop()

    def test_detach_restores_interpreter(self, tmp_path: Path) -> None:
        storage = ensure_storage_dir(tmp_path / "kairos")
        configure_logging(storage)
        interp = _make_fake_interpreter(tmp_path)

        kairos = attach_to_interpreter(
            interp,
            storage_path=storage,
            tick_interval=0.05,
        )
        kairos.stop()  # stop background threads first

        detach_from_interpreter(interp)
        assert not hasattr(interp, "kairos")
        assert not hasattr(interp, "kairos_memory")

    def test_chat_records_assistant_messages_as_events(self, tmp_path: Path) -> None:
        storage = ensure_storage_dir(tmp_path / "kairos")
        configure_logging(storage)
        interp = _make_fake_interpreter(tmp_path)

        kairos = attach_to_interpreter(
            interp,
            storage_path=storage,
            tick_interval=60,  # won't tick during this test
        )

        interp.chat(message="hello")
        interp.chat(message="world")

        events = interp.kairos_memory.load_events()
        kinds = [e.kind for e in events]
        # Should have user_activity events + assistant_message events
        assert any("user_activity" in k for k in kinds)
        assert any("assistant" in k for k in kinds)

        kairos.stop()

    def test_ultraplan_handler_is_registered(self, tmp_path: Path) -> None:
        storage = ensure_storage_dir(tmp_path / "kairos")
        configure_logging(storage)
        interp = _make_fake_interpreter(tmp_path)

        kairos = attach_to_interpreter(
            interp,
            storage_path=storage,
            tick_interval=60,
        )

        from kairos_agent.common import TaskRequest

        # UltraPlan handler should be registered and callable via Coordinator.
        coord = kairos._coordinator
        task = TaskRequest(handler_name="ultraplan.generate",
                           payload={"goal": "test goal"})
        coord.enqueue(task)
        coord.run_pending()

        # Plan should have been persisted
        plans_events = interp.kairos_memory.load_events()
        # Just verify no crash; the plan is in the DB
        plan = interp.kairos_memory.get_plan(1)
        assert plan is not None
        assert plan.goal == "test goal"

        kairos.stop()
