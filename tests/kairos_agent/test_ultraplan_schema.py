"""ULTRAPLAN: schema round-trip + LLM-backed plan generation."""
from __future__ import annotations

import json

from kairos_agent import DummyLLMClient, KairosConfig, MemoryManager
from kairos_agent.ultraplan import Plan, Task, UltraPlan


def test_plan_json_round_trip() -> None:
    plan = Plan(
        goal="launch product",
        tasks=[
            Task(id="t1", description="design", deps=[], priority=2),
            Task(id="t2", description="build", deps=["t1"], priority=5),
            Task(id="t3", description="ship", deps=["t1", "t2"], priority=1),
        ],
    )
    raw = plan.to_dict()
    restored = Plan.from_dict(raw)
    assert restored.goal == plan.goal
    assert len(restored.tasks) == 3
    assert restored.tasks[2].deps == ["t1", "t2"]
    assert restored.tasks[0].priority == 2


def test_generate_parses_json_response(memory: MemoryManager) -> None:
    response = json.dumps({
        "goal": "write blog",
        "tasks": [
            {"id": "t1", "description": "outline", "deps": [], "priority": 3, "status": "pending"},
            {"id": "t2", "description": "draft", "deps": ["t1"], "priority": 5, "status": "pending"},
        ],
    })
    llm = DummyLLMClient([response])
    up = UltraPlan(llm, memory)
    plan = up.generate("write blog")
    assert plan.id is not None  # persisted
    assert len(plan.tasks) == 2
    assert plan.tasks[1].deps == ["t1"]


def test_generate_parses_bullet_list(memory: MemoryManager) -> None:
    llm = DummyLLMClient([
        "Here's a plan:\n- Research the topic\n- Write first draft\n- Proofread\n"
    ])
    up = UltraPlan(llm, memory)
    plan = up.generate("write essay")
    assert len(plan.tasks) == 3
    assert plan.tasks[0].description == "Research the topic"


def test_generate_fallback_on_empty_response() -> None:
    llm = DummyLLMClient([""])
    up = UltraPlan(llm, memory=None)
    plan = up.generate("do something", persist=False)
    assert len(plan.tasks) >= 1
    assert plan.tasks[0].description == "do something"


def test_generate_with_dummy_echo() -> None:
    """Default DummyLLMClient (echo mode) still yields a valid plan."""
    llm = DummyLLMClient()
    up = UltraPlan(llm, memory=None)
    plan = up.generate("launch rocket", persist=False)
    assert len(plan.tasks) >= 1
