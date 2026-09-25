import http.client
import io
import json
import unittest
import urllib.error
from unittest.mock import MagicMock, patch

from verify_pypi_release import verify_release


class VerifyPyPIReleaseTest(unittest.TestCase):
    def test_waits_for_complete_release_after_registry_error(self) -> None:
        wheel = "openai_codex-1.2.3-py3-none-any.whl"
        sdist = "openai_codex-1.2.3.tar.gz"
        reset_response = MagicMock()
        reset_response.__enter__.return_value.read.side_effect = ConnectionResetError(
            "connection reset while reading response"
        )
        responses = [
            urllib.error.URLError("registry unavailable"),
            TimeoutError("registry timed out"),
            http.client.IncompleteRead(b"{", 20),
            reset_response,
            io.StringIO('{"urls":'),
            *[
                io.StringIO(json.dumps(data))
                for data in (
                    None,
                    [],
                    {},
                    {"urls": None},
                    {"urls": {}},
                    {"urls": [None]},
                    {"urls": [{}]},
                    {"urls": [{"filename": 42}]},
                )
            ],
            io.StringIO(json.dumps({"urls": [{"filename": wheel}]})),
            io.StringIO(
                json.dumps({"urls": [{"filename": wheel}, {"filename": sdist}]})
            ),
        ]
        with (
            patch(
                "verify_pypi_release.urllib.request.urlopen", side_effect=responses
            ) as urlopen,
            patch("verify_pypi_release.time.sleep") as sleep,
        ):
            verify_release("openai-codex", "1.2.3")

        self.assertEqual(urlopen.call_count, 15)
        self.assertEqual(sleep.call_count, 14)
        urlopen.assert_called_with(
            "https://pypi.org/pypi/openai-codex/1.2.3/json", timeout=30
        )

    def test_uses_canonical_version_for_lookup_and_artifact_names(self) -> None:
        data = {
            "urls": [
                {"filename": name}
                for name in (
                    "openai_codex-1.2.3b1-py3-none-any.whl",
                    "openai_codex-1.2.3b1.tar.gz",
                )
            ]
        }
        with (
            patch(
                "verify_pypi_release.urllib.request.urlopen",
                return_value=io.StringIO(json.dumps(data)),
            ) as urlopen,
            patch("verify_pypi_release.time.sleep") as sleep,
        ):
            verify_release("openai-codex", "1.2.3b01")
        urlopen.assert_called_once_with(
            "https://pypi.org/pypi/openai-codex/1.2.3b1/json", timeout=30
        )
        sleep.assert_not_called()

    def test_fails_after_bounded_retries_when_runtime_wheels_are_missing(self) -> None:
        with (
            patch(
                "verify_pypi_release.urllib.request.urlopen",
                side_effect=lambda *args, **kwargs: io.StringIO('{"urls": []}'),
            ) as urlopen,
            patch("verify_pypi_release.time.sleep") as sleep,
            self.assertRaisesRegex(SystemExit, "did not become available on PyPI"),
        ):
            verify_release("openai-codex-cli-bin", "1.2.3a4.post5")

        self.assertEqual(urlopen.call_count, 30)
        self.assertEqual(sleep.call_count, 29)


if __name__ == "__main__":
    unittest.main()
