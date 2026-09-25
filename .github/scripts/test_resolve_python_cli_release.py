import copy
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import resolve_python_cli_release as resolver


class ResolvePythonCliReleaseTest(unittest.TestCase):
    def setUp(self) -> None:
        self.revision = "a" * 40
        self.run = {
            "id": 123,
            "path": ".github/workflows/rust-release.yml",
            "event": "push",
            "status": "completed",
            "conclusion": "success",
            "head_repository": {"full_name": "openai/codex"},
            "head_branch": "rust-v1.2.3",
            "head_sha": self.revision,
        }
        self.job = {
            "id": 456,
            "run_attempt": 1,
            "name": "release",
            "conclusion": "success",
        }
        self.commit = {"type": "commit", "sha": self.revision}
        self.release = {
            "id": 789,
            "draft": False,
            "prerelease": False,
            "tag_name": "rust-v1.2.3",
        }
        self.assets = [
            {"name": name, "state": "uploaded", "size": 100}
            for name in [
                "openai_codex_cli_bin-1.2.3-py3-none-macosx_10_9_x86_64.whl",
                "openai_codex_cli_bin-1.2.3-py3-none-macosx_11_0_arm64.whl",
                "openai_codex_cli_bin-1.2.3-py3-none-manylinux_2_17_aarch64.whl",
                "openai_codex_cli_bin-1.2.3-py3-none-manylinux_2_17_x86_64.whl",
                "openai_codex_cli_bin-1.2.3-py3-none-win_amd64.whl",
                "openai_codex_cli_bin-1.2.3-py3-none-win_arm64.whl",
                "codex-package-aarch64-unknown-linux-musl.tar.gz",
                "codex-package-x86_64-unknown-linux-musl.tar.gz",
            ]
        ]
        self.expected = {
            "source_sha": self.revision,
            "release_tag": "rust-v1.2.3",
            "version": "1.2.3",
        }

    def responses(self) -> list:
        return copy.deepcopy(
            [
                self.run,
                {"jobs": [self.job]},
                {"object": self.commit},
                self.release,
                self.assets,
            ]
        )

    def test_resolves_lightweight_and_annotated_tags_to_the_run_revision(self) -> None:
        for annotated in (False, True):
            with self.subTest(annotated=annotated):
                responses = self.responses()
                if annotated:
                    responses.insert(2, {"object": {"type": "tag", "sha": "b" * 40}})
                with patch.object(resolver, "github_api", side_effect=responses):
                    self.assertEqual(
                        resolver.resolve_release("openai/codex", "123"), self.expected
                    )

    def test_skips_cli_prereleases_before_reading_jobs_or_assets(self) -> None:
        for suffix in ("-alpha", "-alpha.1", "-alpha.1.2", "-beta", "-beta.1"):
            with self.subTest(suffix=suffix):
                run = {**self.run, "head_branch": f"rust-v1.2.3{suffix}"}
                with patch.object(resolver, "github_api", return_value=run) as api:
                    self.assertIsNone(resolver.resolve_release("openai/codex", "123"))
                api.assert_called_once_with("repos/openai/codex/actions/runs/123")

    def test_rejects_unrelated_or_incomplete_runs(self) -> None:
        for field, value in (
            ("id", 999),
            ("path", ".github/workflows/sdk.yml"),
            ("event", "pull_request"),
            ("status", "in_progress"),
            ("head_branch", "main"),
            ("head_sha", "main"),
            ("head_repository", {"full_name": "other/codex"}),
        ):
            with self.subTest(field=field, value=value):
                with patch.object(
                    resolver, "github_api", return_value={**self.run, field: value}
                ):
                    with self.assertRaises(ValueError):
                        resolver.resolve_release("openai/codex", "123")

    def test_accepts_failed_ancillary_publisher_and_its_partial_rerun(self) -> None:
        for overall in ("failure", "cancelled"):
            with self.subTest(overall=overall):
                responses = self.responses()
                responses[0]["conclusion"] = overall
                responses[1]["jobs"].extend(
                    [
                        {
                            "id": 999,
                            "run_attempt": 2,
                            "name": "publish-winget",
                            "conclusion": "failure",
                        },
                        {
                            **self.job,
                            "id": 998,
                            "run_attempt": 2,
                            "conclusion": "skipped",
                        },
                    ]
                )
                with patch.object(resolver, "github_api", side_effect=responses):
                    self.assertEqual(
                        resolver.resolve_release("openai/codex", "123"), self.expected
                    )

    def test_requires_success_from_latest_executed_release_job(self) -> None:
        for jobs in (
            [],
            [{**self.job, "conclusion": "skipped"}],
            [{**self.job, "conclusion": "failure"}],
            [{**self.job, "conclusion": "cancelled"}],
            [
                {**self.job, "id": 999, "run_attempt": 2, "conclusion": "failure"},
                self.job,
            ],
        ):
            with self.subTest(jobs=jobs):
                with patch.object(
                    resolver, "github_api", side_effect=[self.run, {"jobs": jobs}]
                ):
                    with self.assertRaisesRegex(ValueError, "successful release job"):
                        resolver.resolve_release("openai/codex", "123")

    def test_reads_all_pages_of_jobs_and_assets(self) -> None:
        responses = self.responses()
        responses.insert(1, {"jobs": [{**self.job, "name": "other"}] * 100})
        responses.insert(-1, [{"name": "other", "state": "uploaded", "size": 1}] * 100)
        with patch.object(resolver, "github_api", side_effect=responses) as api:
            self.assertEqual(
                resolver.resolve_release("openai/codex", "123"), self.expected
            )
        api.assert_any_call(
            "repos/openai/codex/actions/runs/123/jobs?filter=all&per_page=100&page=2"
        )
        api.assert_any_call(
            "repos/openai/codex/releases/789/assets?per_page=100&page=2"
        )

    def test_rejects_a_moved_tag(self) -> None:
        responses = self.responses()
        responses[2] = {"object": {"type": "commit", "sha": "b" * 40}}
        with patch.object(resolver, "github_api", side_effect=responses):
            with self.assertRaisesRegex(ValueError, "no longer matches"):
                resolver.resolve_release("openai/codex", "123")

    def test_bounds_annotated_tag_resolution(self) -> None:
        tag = {"object": {"type": "tag", "sha": "b" * 40}}
        with patch.object(
            resolver,
            "github_api",
            side_effect=[self.run, {"jobs": [self.job]}, *([tag] * 9)],
        ) as api:
            with self.assertRaisesRegex(ValueError, "no longer matches"):
                resolver.resolve_release("openai/codex", "123")
        self.assertEqual(api.call_count, 11)

    def test_requires_a_published_stable_release_for_the_tag(self) -> None:
        for field, value in (
            ("draft", True),
            ("prerelease", True),
            ("tag_name", "rust-v9.9.9"),
        ):
            responses = self.responses()
            responses[3][field] = value
            with self.subTest(field=field):
                with patch.object(resolver, "github_api", side_effect=responses):
                    with self.assertRaisesRegex(ValueError, "published and stable"):
                        resolver.resolve_release("openai/codex", "123")

    def test_requires_all_runtime_inputs_to_be_uploaded_and_nonempty(self) -> None:
        for assets in (
            self.assets[:-1],
            [*self.assets[:-1], {**self.assets[-1], "size": 0}],
            [*self.assets[:-1], {**self.assets[-1], "state": "new"}],
        ):
            with self.subTest(assets=assets):
                responses = self.responses()
                responses[-1] = assets
                with patch.object(resolver, "github_api", side_effect=responses):
                    with self.assertRaisesRegex(ValueError, "missing Python inputs"):
                        resolver.resolve_release("openai/codex", "123")

    def test_rejects_invalid_inputs_before_github_access(self) -> None:
        with patch.object(resolver, "github_api") as api:
            for repository, run_id in (
                ("other/codex", "123"),
                ("openai/codex", "../123"),
            ):
                with self.assertRaises(ValueError):
                    resolver.resolve_release(repository, run_id)
            api.assert_not_called()

    def test_event_and_manual_retry_emit_the_same_revision_and_version(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "output"
            event = Path(directory) / "event.json"
            for automatic in (True, False):
                output.write_text("")
                event.write_text(
                    json.dumps({"workflow_run": self.run} if automatic else {})
                )
                responses = self.responses()[1:] if automatic else self.responses()
                with patch.object(resolver, "github_api", side_effect=responses) as api:
                    resolver.main(
                        [
                            "123",
                            "--repository",
                            "openai/codex",
                            "--event-path",
                            str(event),
                            "--github-output",
                            str(output),
                        ]
                    )
                self.assertEqual(
                    output.read_text(),
                    f"publish=true\nsource_sha={self.revision}\nrelease_tag=rust-v1.2.3\nversion=1.2.3\n",
                )
                if automatic:
                    self.assertNotIn(
                        unittest.mock.call("repos/openai/codex/actions/runs/123"),
                        api.call_args_list,
                    )


if __name__ == "__main__":
    unittest.main()
