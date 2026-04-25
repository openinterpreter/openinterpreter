#!/usr/bin/env python3

from __future__ import annotations

import json
import sys
import urllib.request
from collections import defaultdict
from pathlib import Path


LITELLM_CATALOG_URL = (
    "https://raw.githubusercontent.com/BerriAI/litellm/main/"
    "model_prices_and_context_window.json"
)
REPO_ROOT = Path(__file__).resolve().parents[2]
OUTPUT_PATH = REPO_ROOT / "codex-rs" / "codex-api" / "model_compatibility_catalog.json"
OVERRIDES_PATH = REPO_ROOT / "codex-rs" / "codex-api" / "model_compatibility_overrides.json"


def load_litellm_catalog() -> dict[str, dict]:
    with urllib.request.urlopen(LITELLM_CATALOG_URL, timeout=30) as response:
        return json.load(response)


def load_overrides() -> dict[str, list[str]]:
    return json.loads(OVERRIDES_PATH.read_text())


def has_text_output(metadata: dict) -> bool:
    output_modalities = metadata.get("supported_output_modalities")
    if isinstance(output_modalities, list) and output_modalities:
        return any(modality == "text" for modality in output_modalities)

    mode = str(metadata.get("mode", "")).lower()
    return mode in {"chat", "responses"}


def supports_codex_tool_use(metadata: dict) -> bool:
    return bool(
        metadata.get("supports_function_calling")
        or metadata.get("supports_tool_choice")
    )


def supports_chat_or_responses(metadata: dict) -> bool:
    mode = str(metadata.get("mode", "")).lower()
    if mode in {"chat", "responses"}:
        return True

    endpoints = metadata.get("supported_endpoints")
    if not isinstance(endpoints, list):
        return False

    return any(
        endpoint in {"/v1/chat/completions", "/v1/responses"}
        for endpoint in endpoints
    )


def supported_parameters(metadata: dict) -> list[str]:
    params: list[str] = []
    if supports_codex_tool_use(metadata):
        params.extend(["tools", "tool_choice"])
    if supports_reasoning(metadata):
        params.append("reasoning_effort")
    if metadata.get("supports_web_search"):
        params.append("web_search_options")
    return params


def supported_reasoning_efforts(metadata: dict) -> list[dict[str, str]]:
    if not supports_reasoning(metadata):
        return []

    efforts: list[str] = []
    if metadata.get("supports_none_reasoning_effort"):
        efforts.append("none")
    if metadata.get("supports_minimal_reasoning_effort"):
        efforts.append("minimal")
    efforts.extend(["low", "medium", "high"])
    if metadata.get("supports_xhigh_reasoning_effort"):
        efforts.append("xhigh")

    deduped: list[str] = []
    for effort in efforts:
        if effort not in deduped:
            deduped.append(effort)
    return [{"effort": effort, "description": effort} for effort in deduped]


def supports_reasoning(metadata: dict) -> bool:
    return bool(
        metadata.get("supports_reasoning")
        or metadata.get("supports_minimal_reasoning_effort")
        or metadata.get("supports_none_reasoning_effort")
        or metadata.get("supports_xhigh_reasoning_effort")
    )


def input_modalities(metadata: dict) -> list[str]:
    modalities = metadata.get("supported_modalities")
    if isinstance(modalities, list) and modalities:
        values = [modality for modality in modalities if modality in {"text", "image"}]
        if values:
            return values

    if metadata.get("supports_vision"):
        return ["text", "image"]

    return ["text"]


def context_window(metadata: dict) -> int | None:
    for key in ["max_input_tokens", "max_tokens"]:
        value = metadata.get(key)
        if isinstance(value, int):
            return value
    return None


def visibility(metadata: dict) -> str:
    if (
        not supports_chat_or_responses(metadata)
        or not has_text_output(metadata)
        or not supports_codex_tool_use(metadata)
    ):
        return "hide"
    return "list"


def priority(metadata: dict, visibility_name: str) -> int:
    if visibility_name != "list":
        return 200
    if supports_reasoning(metadata):
        return 40
    if supports_codex_tool_use(metadata):
        return 50
    return 90


def description(metadata: dict) -> str | None:
    tags: list[str] = []
    if supports_chat_or_responses(metadata):
        mode = str(metadata.get("mode", "")).lower()
        if mode == "responses":
            tags.append("Responses")
        else:
            tags.append("Chat-compatible")
    if supports_codex_tool_use(metadata):
        tags.append("Tool calling")
    if supports_reasoning(metadata):
        tags.append("Reasoning effort")
    if metadata.get("supports_web_search"):
        tags.append("Search")
    return " • ".join(tags) if tags else None


def build_entry(model_id: str, metadata: dict, force_hide_ids: set[str]) -> dict:
    visibility_name = visibility(metadata)
    if model_id in force_hide_ids:
        visibility_name = "hide"
    return {
        "id": model_id,
        "aliases": [],
        "description": description(metadata),
        "visibility": visibility_name,
        "supported_parameters": supported_parameters(metadata),
        "supported_reasoning_levels": supported_reasoning_efforts(metadata),
        "supports_parallel_tool_calls": bool(
            metadata.get("supports_parallel_function_calling")
        ),
        "supports_search_tool": bool(metadata.get("supports_web_search")),
        "input_modalities": input_modalities(metadata),
        "context_window": context_window(metadata),
    }


def unique_suffix_aliases(model_ids: list[str]) -> dict[str, list[str]]:
    owners: defaultdict[str, set[str]] = defaultdict(set)
    for model_id in model_ids:
        parts = model_id.split("/")
        for index in range(1, len(parts)):
            owners["/".join(parts[index:])].add(model_id)

    aliases: dict[str, list[str]] = {model_id: [] for model_id in model_ids}
    for alias, model_id_set in owners.items():
        if len(model_id_set) != 1:
            continue
        [owner] = list(model_id_set)
        aliases[owner].append(alias)

    for model_id in model_ids:
        aliases[model_id].sort()

    return aliases


def write_catalog() -> int:
    litellm_catalog = load_litellm_catalog()
    overrides = load_overrides()
    force_hide_ids = set(overrides.get("force_hide_ids", []))
    model_ids = sorted(litellm_catalog)
    aliases = unique_suffix_aliases(model_ids)
    entries = []

    for model_id in model_ids:
        entry = build_entry(model_id, litellm_catalog[model_id], force_hide_ids)
        entry["aliases"] = aliases[model_id]
        entries.append(entry)

    payload = {
        "generated_from": LITELLM_CATALOG_URL,
        "entries": entries,
    }
    OUTPUT_PATH.write_text(json.dumps(payload, indent=2, sort_keys=False) + "\n")
    print(f"Wrote {len(entries)} compatibility entries to {OUTPUT_PATH}")
    return 0


if __name__ == "__main__":
    sys.exit(write_catalog())
