"""Shared infrastructure: interfaces, DTOs, logging, paths."""
from .dto import TaskRequest, TriggerEvent
from .dummy_llm import DummyLLMClient
from .interfaces import Clock, EventBus, LLMClient, SystemClock
from .logger import configure_logging, get_logger
from .paths import DEFAULT_STORAGE_DIR, ensure_storage_dir

__all__ = [
    "TaskRequest",
    "TriggerEvent",
    "DummyLLMClient",
    "Clock",
    "EventBus",
    "LLMClient",
    "SystemClock",
    "configure_logging",
    "get_logger",
    "DEFAULT_STORAGE_DIR",
    "ensure_storage_dir",
]
