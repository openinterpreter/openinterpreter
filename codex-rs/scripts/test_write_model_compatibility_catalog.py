import json
import unittest

import write_model_compatibility_catalog as catalog


class ModelCompatibilityAliasTests(unittest.TestCase):
    def test_suffix_aliases_never_shadow_canonical_ids(self):
        aliases = catalog.unique_suffix_aliases(
            [
                "deepseek-v4-flash",
                "openrouter/openai/gpt-5-codex",
                "tensormesh/deepseek-ai/DeepSeek-V4-Flash",
            ]
        )
        self.assertEqual(
            aliases,
            {
                "deepseek-v4-flash": [],
                "openrouter/openai/gpt-5-codex": ["gpt-5-codex", "openai/gpt-5-codex"],
                "tensormesh/deepseek-ai/DeepSeek-V4-Flash": [
                    "deepseek-ai/DeepSeek-V4-Flash"
                ],
            },
        )

    def test_bundled_aliases_do_not_shadow_canonical_ids(self):
        payload = json.loads(catalog.OUTPUT_PATH.read_text(encoding="utf-8"))
        canonical_ids = {entry["id"].lower() for entry in payload["entries"]}
        shadowing = [
            (entry["id"], alias)
            for entry in payload["entries"]
            for alias in entry["aliases"]
            if alias.lower() in canonical_ids
        ]
        self.assertEqual(shadowing, [])


if __name__ == "__main__":
    unittest.main()
