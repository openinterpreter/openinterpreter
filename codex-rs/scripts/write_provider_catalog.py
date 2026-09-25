#!/usr/bin/env python3

from __future__ import annotations

import argparse
import json
import os
import sys
import urllib.error
import urllib.request
from pathlib import Path


MODELS_DEV_URL = "https://models.dev/api.json"
REPO_ROOT = Path(__file__).resolve().parents[2]
OUTPUT_PATH = REPO_ROOT / "codex-rs" / "model-provider-info" / "provider_catalog.json"
OVERRIDES_PATH = (
    REPO_ROOT / "codex-rs" / "model-provider-info" / "provider_catalog_overrides.json"
)
DEFAULT_SORT_PRIORITY = 100
SUPPORTED_WIRE_APIS = {"chat", "messages", "responses"}
USER_AGENT = "OpenInterpreter/1.0 (+https://github.com/openinterpreter/openinterpreter)"


def load_models_dev_catalog() -> dict[str, dict]:
    request = urllib.request.Request(
        MODELS_DEV_URL,
        headers={"User-Agent": USER_AGENT},
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


def provider_model_allowlist(
    provider_id: str,
    overrides: dict[str, object],
) -> set[str] | None:
    values = overrides.get("provider_model_allowlists", {})
    if not isinstance(values, dict):
        raise SystemExit("provider_model_allowlists must be an object")
    allowlist = values.get(provider_id)
    if allowlist is None:
        return None
    if not isinstance(allowlist, list) or not all(
        isinstance(model_id, str) and model_id for model_id in allowlist
    ):
        raise SystemExit(
            f"provider_model_allowlists.{provider_id} must be a list of non-empty strings"
        )
    if len(set(allowlist)) != len(allowlist):
        raise SystemExit(
            f"provider_model_allowlists.{provider_id} must not contain duplicates"
        )
    return set(allowlist)


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

    source_models = provider.get("models") or {}
    if not isinstance(source_models, dict):
        raise SystemExit(f"models.dev provider {provider_id}.models must be an object")
    model_allowlist = provider_model_allowlist(provider_id, overrides)
    if model_allowlist is not None:
        missing = sorted(model_allowlist - set(source_models))
        if missing:
            raise SystemExit(
                f"provider_model_allowlists.{provider_id} missing from models.dev: "
                + ", ".join(missing)
            )

    env_key_overrides = overrides.get("env_key_overrides", {})
    if not isinstance(env_key_overrides, dict):
        env_key_overrides = {}
    env_keys = provider.get("env") or []
    env_key = env_key_overrides.get(provider_id) or (env_keys[0] if env_keys else None)
    sort_priorities = overrides.get("sort_priorities", {})
    if not isinstance(sort_priorities, dict):
        sort_priorities = {}
    wire_api = wire_api_for_provider(provider_id, provider, overrides)

    models: list[dict] = []
    for priority, (model_id, model) in enumerate(source_models.items()):
        if model_allowlist is not None and model_id not in model_allowlist:
            continue
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
    merge_live_provider_models(provider_id, models, overrides)
    apply_provider_model_additions(provider_id, models, overrides)
    apply_provider_model_overrides(provider_id, models, overrides)
    if model_allowlist is not None:
        models[:] = [model for model in models if model.get("id") in model_allowlist]
        missing = sorted(
            model_allowlist
            - {model["id"] for model in models if isinstance(model.get("id"), str)}
        )
        if missing:
            raise SystemExit(
                f"provider_model_allowlists.{provider_id} models were filtered out: "
                + ", ".join(missing)
            )
        for priority, model in enumerate(models):
            model["priority"] = priority

    return {
        "id": provider_id,
        "name": provider["name"],
        "env_key": env_key,
        "base_url": base_url,
        "wire_api": wire_api,
        "sort_priority": int(sort_priorities.get(provider_id, DEFAULT_SORT_PRIORITY)),
        "models": models,
    }


def merge_live_provider_models(
    provider_id: str,
    models: list[dict],
    overrides: dict[str, object],
) -> None:
    live_sources = overrides.get("live_model_sources", {})
    if not isinstance(live_sources, dict):
        return
    source = live_sources.get(provider_id)
    if not isinstance(source, dict):
        return
    url = source.get("url")
    if not isinstance(url, str) or not url:
        raise SystemExit(f"live_model_sources.{provider_id}.url must be a string")

    headers = {"User-Agent": USER_AGENT}
    auth = source.get("auth")
    if auth == "bearer":
        auth_env = source.get("auth_env", [])
        token = first_env_value(auth_env)
        if token is None:
            raise SystemExit(
                f"live_model_sources.{provider_id} requires one of these env vars: "
                f"{format_env_names(auth_env)}"
            )
        headers["Authorization"] = f"Bearer {token}"
    elif auth is not None:
        raise SystemExit(
            f"unsupported live model source auth for {provider_id}: {auth}"
        )

    try:
        request = urllib.request.Request(url, headers=headers)
        with urllib.request.urlopen(request, timeout=30) as response:
            payload = json.load(response)
    except (OSError, urllib.error.URLError, urllib.error.HTTPError) as exc:
        raise SystemExit(f"live_model_sources.{provider_id} failed: {exc}") from exc

    existing = {model["id"] for model in models if isinstance(model.get("id"), str)}
    next_priority = (
        max(
            (int(model.get("priority", -1)) for model in models),
            default=-1,
        )
        + 1
    )
    live_ids = live_model_ids(payload)
    if not live_ids:
        raise SystemExit(f"live_model_sources.{provider_id} returned no models")
    if source.get("filter_existing_to_live_ids") is True:
        live_id_set = set(live_ids)
        models[:] = [
            model
            for model in models
            if isinstance(model.get("id"), str) and model["id"] in live_id_set
        ]
        for priority, model in enumerate(models):
            model["priority"] = priority
        existing = {model["id"] for model in models if isinstance(model.get("id"), str)}
        next_priority = len(models)
    if source.get("append_new_models") is False:
        return
    for model_id in live_ids:
        if model_id in existing:
            continue
        models.append(default_live_model_entry(model_id, next_priority))
        existing.add(model_id)
        next_priority += 1


def first_env_value(env_names: object) -> str | None:
    if not isinstance(env_names, list):
        raise SystemExit("live model source auth_env must be a list")
    for env_name in env_names:
        if not isinstance(env_name, str):
            continue
        value = os.environ.get(env_name)
        if value:
            return value
    return None


def format_env_names(env_names: object) -> str:
    if not isinstance(env_names, list):
        raise SystemExit("live model source auth_env must be a list")
    values = [
        env_name for env_name in env_names if isinstance(env_name, str) and env_name
    ]
    return ", ".join(values) if values else "<none configured>"


def live_model_ids(payload: object) -> list[str]:
    if not isinstance(payload, dict):
        return []
    data = payload.get("data")
    if not isinstance(data, list):
        return []
    values = []
    for item in data:
        if not isinstance(item, dict):
            continue
        model_id = item.get("id")
        if isinstance(model_id, str) and model_id:
            values.append(model_id)
    return values


def default_live_model_entry(model_id: str, priority: int) -> dict:
    return {
        "id": model_id,
        "display_name": model_id.replace("-", " ").replace(".", ".").title(),
        "description": "Live provider model",
        "reasoning": False,
        "input_modalities": ["text"],
        "context_window": None,
        "priority": priority,
    }


def apply_provider_model_overrides(
    provider_id: str,
    models: list[dict],
    overrides: dict[str, object],
) -> None:
    provider_overrides = overrides.get("provider_model_metadata_overrides", {})
    if not isinstance(provider_overrides, dict):
        return
    model_overrides = provider_overrides.get(provider_id)
    if not isinstance(model_overrides, dict):
        return
    for model in models:
        model_id = model.get("id")
        if not isinstance(model_id, str):
            continue
        metadata = model_overrides.get(model_id)
        if isinstance(metadata, dict):
            model.update(metadata)


def apply_provider_model_additions(
    provider_id: str,
    models: list[dict],
    overrides: dict[str, object],
) -> None:
    provider_additions = overrides.get("provider_model_additions", {})
    if not isinstance(provider_additions, dict):
        return
    additions = provider_additions.get(provider_id)
    if not isinstance(additions, list):
        return

    existing = {model["id"] for model in models if isinstance(model.get("id"), str)}
    next_priority = (
        max(
            (int(model.get("priority", -1)) for model in models),
            default=-1,
        )
        + 1
    )
    for addition in additions:
        if not isinstance(addition, dict):
            continue
        model_id = addition.get("id")
        if not isinstance(model_id, str) or not model_id:
            raise SystemExit(
                f"provider_model_additions.{provider_id} entries need an id"
            )
        if model_id in existing:
            continue
        input_modalities = addition.get("input_modalities", ["text"])
        if not isinstance(input_modalities, list) or not all(
            isinstance(modality, str) and modality for modality in input_modalities
        ):
            raise SystemExit(
                f"provider_model_additions.{provider_id}.{model_id}.input_modalities "
                "must be a list of non-empty strings"
            )
        context_window = addition.get("context_window")
        if context_window is not None and not isinstance(context_window, int):
            raise SystemExit(
                f"provider_model_additions.{provider_id}.{model_id}.context_window "
                "must be an integer or null"
            )
        priority = addition.get("priority", next_priority)
        if not isinstance(priority, int):
            raise SystemExit(
                f"provider_model_additions.{provider_id}.{model_id}.priority "
                "must be an integer"
            )
        models.append(
            {
                "id": model_id,
                "display_name": addition.get("display_name") or model_id,
                "description": addition.get("description"),
                "reasoning": bool(addition.get("reasoning")),
                "input_modalities": input_modalities,
                "context_window": context_window,
                "priority": priority,
            }
        )
        existing.add(model_id)
        next_priority = max(next_priority, priority + 1)


def additional_provider_entries(overrides: dict[str, object]) -> list[dict]:
    values = overrides.get("additional_provider_entries", [])
    if not isinstance(values, list):
        raise SystemExit("additional_provider_entries must be a list")
    entries = []
    for value in values:
        if not isinstance(value, dict):
            raise SystemExit("additional_provider_entries entries must be objects")
        required = {"id", "name", "base_url", "wire_api", "models", "sort_priority"}
        missing = sorted(field for field in required if field not in value)
        if missing:
            raise SystemExit(
                "additional provider entry is missing: " + ", ".join(missing)
            )
        entries.append(value)
    return entries


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


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Regenerate the bundled hosted-provider catalog."
    )
    parser.add_argument(
        "--provider",
        action="append",
        dest="provider_ids",
        metavar="ID",
        help=(
            "replace only this provider in the existing catalog; repeat for multiple "
            "providers"
        ),
    )
    return parser.parse_args()


def write_catalog(selected_provider_ids: set[str] | None = None) -> int:
    models_dev_catalog = load_models_dev_catalog()
    overrides = load_overrides()
    generated_providers = []
    for provider_id, provider in sorted(models_dev_catalog.items()):
        if (
            selected_provider_ids is not None
            and provider_id not in selected_provider_ids
        ):
            continue
        if not isinstance(provider, dict):
            continue
        if not should_include_provider(provider_id, provider, overrides):
            continue

        entry = build_provider_entry(provider_id, provider, overrides)
        if entry["models"]:
            generated_providers.append(entry)

    generated_ids = {provider["id"] for provider in generated_providers}
    if selected_provider_ids is not None:
        missing = selected_provider_ids - generated_ids
        if missing:
            raise SystemExit(
                "selected providers were not generated: " + ", ".join(sorted(missing))
            )
        existing_payload = json.loads(OUTPUT_PATH.read_text())
        providers = [
            provider
            for provider in existing_payload.get("providers", [])
            if provider.get("id") not in selected_provider_ids
        ]
        providers.extend(generated_providers)
    else:
        providers = generated_providers
        providers.extend(additional_provider_entries(overrides))
    providers.sort(
        key=lambda provider: (
            int(provider["sort_priority"]),
            str(provider["name"]).lower(),
        )
    )

    payload = {
        "generated_from": generated_from(overrides),
        "providers": providers,
    }
    OUTPUT_PATH.write_text(json.dumps(payload, indent=2) + "\n")
    print(f"Wrote {len(providers)} provider entries to {OUTPUT_PATH}")
    return 0


def generated_from(overrides: dict[str, object]) -> str | list[str]:
    sources = [MODELS_DEV_URL]
    live_sources = overrides.get("live_model_sources", {})
    if isinstance(live_sources, dict):
        for source in live_sources.values():
            if isinstance(source, dict):
                url = source.get("url")
                if isinstance(url, str) and url:
                    sources.append(url)
    return sources[0] if len(sources) == 1 else sources


if __name__ == "__main__":
    args = parse_args()
    selected_provider_ids = set(args.provider_ids) if args.provider_ids else None
    sys.exit(write_catalog(selected_provider_ids))
