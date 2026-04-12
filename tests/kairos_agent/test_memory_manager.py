"""Memory persistence: events round-trip, profile, plans."""
from __future__ import annotations

from kairos_agent.memory import MemoryManager, PlanRecord, task_record


def test_save_and_load_events(memory: MemoryManager) -> None:
    ids = [
        memory.save_event("user_message", {"text": "hi"}),
        memory.save_event("tool_call", {"tool": "shell"}),
        memory.save_event("decision", {"why": "curiosity"}),
    ]
    assert len(ids) == 3 and all(isinstance(i, int) for i in ids)

    events = memory.load_events()
    assert len(events) == 3
    kinds = {e.kind for e in events}
    assert kinds == {"user_message", "tool_call", "decision"}


def test_mark_consolidated_filters_correctly(memory: MemoryManager) -> None:
    memory.save_event("a", {})
    memory.save_event("b", {})
    memory.save_event("c", {})

    pending = memory.load_events(consolidated=False)
    assert len(pending) == 3

    memory.mark_events_consolidated([e.id for e in pending])
    assert memory.load_events(consolidated=False) == []
    assert len(memory.load_events(consolidated=True)) == 3


def test_profile_round_trip(memory: MemoryManager) -> None:
    profile = memory.update_profile(name="Rima", goal="ship kairos")
    assert profile.data["name"] == "Rima"

    reloaded = memory.load_profile()
    assert reloaded.data["name"] == "Rima"
    assert reloaded.data["goal"] == "ship kairos"


def test_plan_round_trip_with_dependencies(memory: MemoryManager) -> None:
    plan = PlanRecord(
        id=None,
        goal="write blog post",
        tasks=[
            task_record("t1", "outline", priority=2),
            task_record("t2", "draft", deps=["t1"]),
            task_record("t3", "review", deps=["t2"], priority=3),
        ],
    )
    plan_id = memory.save_plan(plan)
    assert plan_id == plan.id

    loaded = memory.get_plan(plan_id)
    assert loaded is not None
    assert loaded.goal == "write blog post"
    assert [t.id for t in loaded.tasks] == ["t1", "t2", "t3"]
    assert loaded.tasks[1].deps == ["t1"]
    assert loaded.tasks[0].priority == 2


def test_update_plan_task_status(memory: MemoryManager) -> None:
    plan_id = memory.save_plan(PlanRecord(
        id=None,
        goal="x",
        tasks=[task_record("t1", "first"), task_record("t2", "second", deps=["t1"])],
    ))
    memory.update_plan_task(plan_id, "t1", {"status": "done"})
    loaded = memory.get_plan(plan_id)
    assert loaded.tasks[0].status == "done"
    assert loaded.tasks[1].status == "pending"
