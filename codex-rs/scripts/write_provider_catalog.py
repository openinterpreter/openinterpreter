#!/usr/bin/env python3

from __future__ import annotations

import json
import sys
import urllib.request
from pathlib import Path


MODELS_DEV_URL = "https://models.dev/api.json"
REPO_ROOT = Path(__file__).resolve().parents[2]
OUTPUT_PATH = (
    REPO_ROOT / "codex-rs" / "model-provider-info" / "provider_catalog.json"
)
OVERRIDES_PATH = (
    REPO_ROOT
    / "codex-rs"
    / "model-provider-info"
    / "provider_catalog_overrides.json"
)
DEFAULT_SORT_PRIORITY = 100
SUPPORTED_WIRE_APIS = {"chat", "messages", "responses"}


def load_models_dev_catalog() -> dict[str, dict]:
    request = urllib.request.Request(
        MODELS_DEV_URL,
        headers={"User-Agent": "OpenInterpreter/1.0 (+https://github.com/KillianLucas/open-interpreter-next)"},
    )
    with urllib.request.urlopen(request, timeout=30) as response:
        return json.load(response)


def load_overrides() -> dict[str, object]:
    return json.loads(OVERRIDES_PATH.read_text())


def supported_provider_npm_packages(overrides: dict[str, object]) -> set[str]:
    values = overrides.get("supported_provider_npm_packages", [])
    if not isinstance(values, list):
        raise SystemExit("supported_provider_npm_packages must be a list")
    return {value for value in values if isinstance(value, str) and value}


def included_provider_ids(overrides: dict[str, object]) -> set[str]:
    values = overrides.get("include_provider_ids", [])
    if not isinstance(values, list):
        raise SystemExit("include_provider_ids must be a list")
    return {value for value in values if isinstance(value, str) and value}


def excluded_provider_ids(overrides: dict[str, object]) -> set[str]:
    values = overrides.get("exclude_provider_ids", [])
    if not isinstance(values, list):
        raise SystemExit("exclude_provider_ids must be a list")
    return {value for value in values if isinstance(value, str) and value}


def model_description(metadata: dict) -> str | None:
    parts: list[str] = []
    family = metadata.get("family")
    if isinstance(family, str) and family:
        parts.append(family)
    if metadata.get("reasoning"):
        parts.append("Reasoning")
    if metadata.get("tool_call"):
        parts.append("Tool calling")
    modalities = metadata.get("modalities") or {}
    input_modalities = modalities.get("input") or []
    if "image" in input_modalities:
        parts.append("Image input")
    if "pdf" in input_modalities:
        parts.append("PDF input")
    if "video" in input_modalities:
        parts.append("Video input")
    return " • ".join(parts) or None


def input_modalities(metadata: dict) -> list[str]:
    modalities = metadata.get("modalities") or {}
    inputs = modalities.get("input") or []
    values = ["text"]
    if "image" in inputs:
        values.append("image")
    return values


def context_window(metadata: dict) -> int | None:
    limit = metadata.get("limit")
    if isinstance(limit, dict):
        context = limit.get("context")
        if isinstance(context, int):
            return context
    return None


def include_model(metadata: dict) -> bool:
    modalities = metadata.get("modalities") or {}
    outputs = modalities.get("output") or []
    has_text_output = not outputs or "text" in outputs
    return bool(metadata.get("tool_call")) and has_text_output


def build_provider_entry(
    provider_id: str,
    provider: dict,
    overrides: dict[str, object],
) -> dict:
    api_overrides = overrides.get("api_base_url_overrides", {})
    if not isinstance(api_overrides, dict):
        api_overrides = {}
    base_url = provider.get("api") or api_overrides.get(provider_id)
    if not isinstance(base_url, str) or not base_url:
        raise SystemExit(f"missing api/base_url for provider {provider_id}")

    env_keys = provider.get("env") or []
    env_key = env_keys[0] if env_keys else None
    sort_priorities = overrides.get("sort_priorities", {})
    if not isinstance(sort_priorities, dict):
        sort_priorities = {}
    wire_api = wire_api_for_provider(provider_id, provider, overrides)

    models: list[dict] = []
    for priority, (model_id, model) in enumerate((provider.get("models") or {}).items()):
        if not isinstance(model, dict) or not include_model(model):
            continue
        models.append(
            {
                "id": model_id,
                "display_name": model.get("name") or model_id,
                "description": model_description(model),
                "reasoning": bool(model.get("reasoning")),
                "input_modalities": input_modalities(model),
                "context_window": context_window(model),
                "priority": priority,
            }
        )

    return {
        "id": provider_id,
        "name": provider["name"],
        "env_key": env_key,
        "base_url": base_url,
        "wire_api": wire_api,
        "sort_priority": int(sort_priorities.get(provider_id, DEFAULT_SORT_PRIORITY)),
        "models": models,
    }


def wire_api_for_provider(
    provider_id: str,
    provider: dict,
    overrides: dict[str, object],
) -> str:
    wire_api_overrides = overrides.get("wire_api_overrides", {})
    if not isinstance(wire_api_overrides, dict):
        wire_api_overrides = {}

    wire_api = wire_api_overrides.get(provider_id)
    if wire_api is None and provider.get("npm") == "@ai-sdk/anthropic":
        wire_api = "messages"
    if wire_api is None:
        wire_api = "chat"
    if wire_api not in SUPPORTED_WIRE_APIS:
        raise SystemExit(
            f"unsupported wire_api override for provider {provider_id}: {wire_api}"
        )
    return wire_api


def should_include_provider(
    provider_id: str,
    provider: dict,
    overrides: dict[str, object],
) -> bool:
    if provider_id in excluded_provider_ids(overrides):
        return False

    api_overrides = overrides.get("api_base_url_overrides", {})
    if not isinstance(api_overrides, dict):
        api_overrides = {}
    base_url = provider.get("api") or api_overrides.get(provider_id)
    if not isinstance(base_url, str) or not base_url:
        return False
    if "${" in base_url:
        return False

    normalized_base_url = base_url.lower()
    if "localhost" in normalized_base_url or "127.0.0.1" in normalized_base_url:
        return False

    if provider_id in included_provider_ids(overrides):
        return True

    provider_npm = provider.get("npm")
    supported_npm_packages = supported_provider_npm_packages(overrides)
    return isinstance(provider_npm, str) and provider_npm in supported_npm_packages


def write_catalog() -> int:
    models_dev_catalog = load_models_dev_catalog()
    overrides = load_overrides()
    providers = []
    for provider_id, provider in sorted(models_dev_catalog.items()):
        if not isinstance(provider, dict):
            continue
        if not should_include_provider(provider_id, provider, overrides):
            continue

        entry = build_provider_entry(provider_id, provider, overrides)
        if entry["models"]:
            providers.append(entry)
    providers.sort(
        key=lambda provider: (
            int(provider["sort_priority"]),
            str(provider["name"]).lower(),
        )
    )

    payload = {
        "generated_from": MODELS_DEV_URL,
        "providers": providers,
    }
    OUTPUT_PATH.write_text(json.dumps(payload, indent=2) + "\n")
    print(f"Wrote {len(providers)} provider entries to {OUTPUT_PATH}")
    return 0


if __name__ == "__main__":
    sys.exit(write_catalog())
