#!/usr/bin/env python3

import argparse
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from codex_package.cli import parse_package_version
from codex_package.cli import main


class LlamaServerPackageInputTest(unittest.TestCase):
    def test_open_interpreter_requires_server_before_build(self) -> None:
        args = argparse.Namespace(
            variant="open-interpreter",
            target="x86_64-unknown-linux-gnu",
            llama_server_bin=None,
        )
        with patch("codex_package.cli.parse_args", return_value=args), patch(
            "codex_package.cli.build_source_binaries"
        ) as build:
            with self.assertRaisesRegex(RuntimeError, "require --llama-server-bin"):
                main()
            build.assert_not_called()


class PackageVersionTest(unittest.TestCase):
    def test_accepts_release_prerelease_and_build_versions(self) -> None:
        for version in (
            "0.0.0",
            "1.2.3",
            "0.0.0-internal.deadbeef",
            "1.2.3-alpha.1+build.01",
            "18446744073709551615.0.0",
        ):
            with self.subTest(version=version):
                self.assertEqual(parse_package_version(version), version)

    def test_rejects_versions_the_runtime_cannot_parse(self) -> None:
        for version in (
            "",
            "1",
            "1.2",
            "1.2.3.4",
            "v1.2.3",
            "01.2.3",
            "1.02.3",
            "1.2.03",
            "1.2.3-",
            "1.2.3-alpha..1",
            "1.2.3-01",
            "1.2.3+",
            "1.2.3+build..1",
            "18446744073709551616.0.0",
        ):
            with self.subTest(version=version):
                with self.assertRaises(argparse.ArgumentTypeError):
                    parse_package_version(version)


if __name__ == "__main__":
    unittest.main()
