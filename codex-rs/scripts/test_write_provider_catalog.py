import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import write_provider_catalog as catalog


class ProviderCatalogRefreshTests(unittest.TestCase):
    def test_exclusions_apply_after_source_and_additions(self):
        overrides = {
            "provider_model_additions": {
                "example": [{"id": "new", "reasoning": True}],
            },
            "provider_model_exclusions": {"example": ["retired", "new"]},
        }
        provider = {
            "name": "Example",
            "api": "https://example.com/v1",
            "models": {
                "retired": {"tool_call": True},
                "current": {"tool_call": True},
            },
        }
        entry = catalog.build_provider_entry("example", provider, overrides)
        self.assertEqual([model["id"] for model in entry["models"]], ["current"])

    def test_offline_refresh_is_deterministic_and_provider_scoped(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "provider_catalog.json"
            payload = {
                "generated_from": "existing-source",
                "providers": [
                    {
                        "id": "example",
                        "models": [{"id": "retired", "priority": 0}],
                    },
                    {
                        "id": "untouched",
                        "models": [{"id": "retired", "priority": 0}],
                    },
                ],
            }
            output.write_text(json.dumps(payload), encoding="utf-8")
            overrides = {
                "provider_model_additions": {"example": [{"id": "new"}]},
                "provider_model_exclusions": {"example": ["retired"]},
            }
            with (
                patch.object(catalog, "OUTPUT_PATH", output),
                patch.object(catalog, "load_overrides", return_value=overrides),
                patch.object(
                    catalog,
                    "load_models_dev_catalog",
                    side_effect=AssertionError("network"),
                ),
            ):
                catalog.refresh_existing_catalog({"example"})
                first = output.read_bytes()
                catalog.refresh_existing_catalog({"example"})
                self.assertEqual(output.read_bytes(), first)
            refreshed = json.loads(first)
            self.assertEqual(refreshed["generated_from"], "existing-source")
            self.assertEqual(
                [
                    [model["id"] for model in entry["models"]]
                    for entry in refreshed["providers"]
                ],
                [["new"], ["retired"]],
            )

    def test_bundled_choices_and_docs_agree(self):
        payload = json.loads(catalog.OUTPUT_PATH.read_text(encoding="utf-8"))
        providers = {
            entry["id"]: {model["id"] for model in entry["models"]}
            for entry in payload["providers"]
        }
        overrides = catalog.load_overrides()
        for provider_id, excluded in overrides["provider_model_exclusions"].items():
            self.assertTrue(providers[provider_id].isdisjoint(excluded), provider_id)
        for provider_id, expected in {
            "anthropic": {
                "claude-fable-5-1",
                "claude-opus-5-5",
                "claude-sonnet-5",
                "claude-haiku-4-5-20251001",
            },
            "kimi-for-coding": {"k3-256k", "kimi-for-coding"},
            "alibaba": {"qwen3.8-max", "qwen3.8-flash", "qwen3.7-flash"},
            "deepseek": {"deepseek-flash", "deepseek-v4-pro"},
        }.items():
            self.assertTrue(expected <= providers[provider_id], provider_id)
        for relative_path in ("docs/models.md", "docs/zh/models.md"):
            documentation = (catalog.REPO_ROOT / relative_path).read_text(
                encoding="utf-8"
            )
            self.assertNotIn("gpt-5.1-codex", documentation)
            self.assertIn('model = "gpt-6-sol"', documentation)
            for model_id in (
                "claude-fable-5-1",
                "claude-opus-5-5",
                "claude-sonnet-5",
                "deepseek-flash",
                "deepseek-v4-pro",
            ):
                self.assertIn(f"`{model_id}`", documentation)
        for relative_path in ("docs/deepseek.md", "docs/zh/deepseek.md"):
            documentation = (catalog.REPO_ROOT / relative_path).read_text(
                encoding="utf-8"
            )
            self.assertIn("`deepseek-flash`", documentation)
            self.assertIn("`deepseek-v4-pro`", documentation)
            self.assertIn("`deepseek-v4-flash`", documentation)


if __name__ == "__main__":
    unittest.main()
