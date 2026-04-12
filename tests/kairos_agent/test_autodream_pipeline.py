"""AutoDream pipeline: consolidation with mocked LLM."""
from __future__ import annotations

from kairos_agent import AutoDream, DummyLLMClient, KairosConfig, MemoryManager
from kairos_agent.autodream import Summarizer


def test_consolidation_pipeline(config: KairosConfig, memory: MemoryManager) -> None:
    llm = DummyLLMClient(["FAKE_SUMMARY"])
    config.llm_client = llm
    for i in range(5):
        memory.save_event("msg", {"i": i})
    ad = AutoDream(config, memory)
    report = ad.run()
    assert report["events_consolidated"] == 5
    assert report["summary"] == "FAKE_SUMMARY"
    profile = memory.load_profile()
    assert profile.data["last_summary"] == "FAKE_SUMMARY"
    assert len(profile.data.get("summary_history", [])) == 1
    assert memory.load_events(consolidated=False) == []


def test_consolidation_with_no_events(config: KairosConfig,
                                      memory: MemoryManager) -> None:
    ad = AutoDream(config, memory)
    report = ad.run()
    assert report["events_consolidated"] == 0
    assert report["summary"] == ""


def test_fallback_summarizer_without_llm(config: KairosConfig,
                                          memory: MemoryManager) -> None:
    config.llm_client = None
    for i in range(3):
        memory.save_event("tool_call", {"tool": "shell"})
    summarizer = Summarizer(llm=None)
    ad = AutoDream(config, memory, summarizer=summarizer)
    report = ad.run()
    assert report["events_consolidated"] == 3
    assert "[fallback summary]" in report["summary"]
    assert "tool_call=3" in report["summary"]
