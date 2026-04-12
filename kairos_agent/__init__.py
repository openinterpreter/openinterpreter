"""Kairos Agent — autonomous agent framework (standalone, host-agnostic).

The package is organized as a small set of cooperating modules:

    common/        — interfaces, DTOs, logging, filesystem helpers
    memory/        — persistent storage of events, profile, plans
    coordinator/   — priority queue + locks that route work between modules
    kairos/        — the persistent tick loop, state, and triggers
    autodream/     — periodic memory consolidation (LLM-backed)
    ultraplan/     — goal → structured plan decomposition
    normalizer/    — style + privacy helpers (stub at MVP)
    adapters/      — host integrations (open-interpreter, ...) — Phase 2

Public surface is intentionally narrow: instantiate KairosConfig, then build
MemoryManager, Coordinator, KairosCore from it. See README / plan for usage.
"""
from .autodream import AutoDream, Summarizer
from .common import (
    Clock,
    DummyLLMClient,
    EventBus,
    LLMClient,
    SystemClock,
    TaskRequest,
    TriggerEvent,
    configure_logging,
    get_logger,
)
from .config import KairosConfig
from .coordinator import Coordinator
from .kairos import InactivityTrigger, KairosCore, TimeTrigger, Trigger
from .memory import MemoryManager
from .normalizer import format_code, redact_pii
from .ultraplan import Plan, Task, UltraPlan

__all__ = [
    # config + interfaces
    "KairosConfig",
    "LLMClient",
    "DummyLLMClient",
    "Clock",
    "SystemClock",
    "EventBus",
    "TaskRequest",
    "TriggerEvent",
    "configure_logging",
    "get_logger",
    # core building blocks
    "MemoryManager",
    "Coordinator",
    "KairosCore",
    "Trigger",
    "TimeTrigger",
    "InactivityTrigger",
    # services
    "AutoDream",
    "Summarizer",
    "UltraPlan",
    "Plan",
    "Task",
    # utilities
    "redact_pii",
    "format_code",
]
