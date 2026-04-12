"""Top-level configuration object passed to every Kairos subsystem."""
from __future__ import annotations

import logging
from dataclasses import dataclass, field
from pathlib import Path

from .common.interfaces import Clock, EventBus, LLMClient, SystemClock
from .common.paths import DEFAULT_STORAGE_DIR


@dataclass
class KairosConfig:
    storage_path: Path | str = DEFAULT_STORAGE_DIR
    tick_interval: float = 30.0  # seconds between Kairos tick loop iterations
    log_level: int = logging.INFO
    log_to_console: bool = False
    llm_client: LLMClient | None = None  # required by AutoDream + UltraPlan
    clock: Clock = field(default_factory=SystemClock)
    event_bus: EventBus | None = None

    def __post_init__(self) -> None:
        self.storage_path = Path(self.storage_path).expanduser()
