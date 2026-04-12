"""ULTRAPLAN — goal → structured plan.

The LLM is prompted to emit JSON conforming to the Plan schema. A regex-based
fallback parses simple bullet lists, so the deterministic DummyLLMClient still
yields a usable plan in tests.
"""
from __future__ import annotations

import json
import re
import uuid

from ..common.interfaces import LLMClient
from ..common.logger import get_logger
from ..memory.manager import MemoryManager
from ..memory.schemas import PlanRecord, TaskRecord
from .schemas import Plan, Task


_log = get_logger("ultraplan")


_PROMPT_TEMPLATE = """You are a planning assistant. Decompose the user's goal
into a small set of concrete, sequenced sub-tasks (3 to 8 items). Reply with a
single JSON object — no prose around it — matching this exact schema:

{{
  "goal": "<verbatim goal>",
  "tasks": [
    {{
      "id": "t1",
      "description": "...",
      "deps": [],
      "priority": 5,
      "status": "pending"
    }}
  ]
}}

Rules:
- Task ids are short slugs (t1, t2, ...).
- "deps" lists ids of tasks that must complete first; use [] when none.
- "priority" is an integer from 1 (urgent) to 10 (optional).
- "status" is always "pending".

Goal: {goal}

JSON:"""


_BULLET_RE = re.compile(r"^\s*(?:[-*]|\d+[.)])\s+(.+?)\s*$")
_JSON_OBJECT_RE = re.compile(r"\{.*\}", re.DOTALL)


class UltraPlan:
    def __init__(self, llm: LLMClient, memory: MemoryManager | None = None):
        self._llm = llm
        self._memory = memory

    def generate(self, goal: str, persist: bool = True) -> Plan:
        prompt = _PROMPT_TEMPLATE.format(goal=goal)
        try:
            raw = self._llm.complete(prompt)
        except Exception as exc:  # noqa: BLE001
            _log.exception("ultraplan.llm_error", extra={"err": str(exc)})
            raw = ""

        plan = self._parse(raw, goal)
        if persist and self._memory is not None:
            record = PlanRecord(
                id=None,
                goal=plan.goal,
                tasks=[
                    TaskRecord(
                        id=t.id,
                        description=t.description,
                        deps=list(t.deps),
                        priority=t.priority,
                        status=t.status,
                    )
                    for t in plan.tasks
                ],
                status=plan.status,
                created_at=plan.created_at,
            )
            new_id = self._memory.save_plan(record)
            plan.id = new_id
        _log.info("ultraplan.generated", extra={
            "goal": goal[:80], "task_count": len(plan.tasks),
        })
        return plan

    # ----- parsing helpers --------------------------------------------------

    def _parse(self, raw: str, goal: str) -> Plan:
        # 1. Try to find and parse a JSON object inside the response.
        match = _JSON_OBJECT_RE.search(raw or "")
        if match is not None:
            try:
                obj = json.loads(match.group(0))
                if isinstance(obj, dict) and "tasks" in obj:
                    obj.setdefault("goal", goal)
                    return Plan.from_dict(obj)
            except (json.JSONDecodeError, KeyError, ValueError):
                pass

        # 2. Fall back to bullet-list extraction so DummyLLMClient still works.
        bullets = self._extract_bullets(raw or "")
        if bullets:
            tasks = [
                Task(id=f"t{i + 1}", description=text)
                for i, text in enumerate(bullets)
            ]
            return Plan(goal=goal, tasks=tasks)

        # 3. Last resort: a single task that just restates the goal so the
        # plan is never empty (downstream code can rely on len(tasks) >= 1).
        fallback_id = f"t1-{uuid.uuid4().hex[:6]}"
        return Plan(
            goal=goal,
            tasks=[Task(id=fallback_id, description=goal, priority=5)],
        )

    @staticmethod
    def _extract_bullets(text: str) -> list[str]:
        out: list[str] = []
        for line in text.splitlines():
            m = _BULLET_RE.match(line)
            if m:
                out.append(m.group(1).strip())
        return out
