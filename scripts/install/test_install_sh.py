#!/usr/bin/env python3

import hashlib
import json
import os
import platform
import stat
import subprocess
import tarfile
import tempfile
import textwrap
import unittest
from pathlib import Path
from shlex import quote


REPO_ROOT = Path(__file__).resolve().parents[2]
INSTALL_SH = REPO_ROOT / "scripts" / "install" / "install.sh"


def current_installer_target() -> str:
    machine = platform.machine().lower()
    arch = "aarch64" if machine in {"arm64", "aarch64"} else "x86_64"
    system = platform.system()
    if system == "Darwin":
        return f"{arch}-apple-darwin"
    if system == "Linux":
        return f"{arch}-unknown-linux-musl"
    raise unittest.SkipTest(f"install.sh does not support {system}")


def write_executable(path: Path, body: str) -> None:
    path.write_text(body, encoding="utf-8")
    path.chmod(path.stat().st_mode | stat.S_IXUSR)


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


class InstallShLatestResolutionTests(unittest.TestCase):
    def run_installer_with_release_list(
        self,
        release_list: str,
        *,
        repo: str = "openinterpreter/openinterpreter",
        product_name: str = "Open Interpreter",
        package_asset_stem: str = "open-interpreter-package",
        command_name: str = "interpreter",
    ) -> subprocess.CompletedProcess[str]:
        with tempfile.TemporaryDirectory() as tmpdir:
            tmp = Path(tmpdir)
            release_list_path = tmp / "releases.json"
            release_list_path.write_text(release_list, encoding="utf-8")
            fake_bin = tmp / "bin"
            fake_bin.mkdir()
            fake_curl = fake_bin / "curl"
            fake_curl.write_text(
                textwrap.dedent(
                    f"""\
                    #!/usr/bin/env bash
                    url="${{@:$#}}"
                    case "$url" in
                      */releases?per_page=100)
                        cat {quote(str(release_list_path))}
                        ;;
                      */releases/tags/rust-v0.2.0)
                        printf '{{"assets":[]}}\\n'
                        ;;
                      */releases/tags/rust-v0.135.0)
                        printf '{{"assets":[]}}\\n'
                        ;;
                      *)
                        echo "unexpected curl URL: $url" >&2
                        exit 42
                        ;;
                    esac
                    """
                ),
                encoding="utf-8",
            )
            fake_curl.chmod(fake_curl.stat().st_mode | stat.S_IXUSR)

            env = {
                **os.environ,
                "PATH": f"{fake_bin}{os.pathsep}{os.environ['PATH']}",
                "CODEX_GITHUB_REPO": repo,
                "CODEX_INSTALL_PRODUCT_NAME": product_name,
                "CODEX_PACKAGE_ASSET_STEM": package_asset_stem,
                "CODEX_COMMAND_NAME": command_name,
                "CODEX_RELEASE_TAG_PREFIX": "rust-v",
                "CODEX_NON_INTERACTIVE": "1",
                "CODEX_HOME": str(tmp / "home"),
                "CODEX_INSTALL_DIR": str(tmp / "install-bin"),
            }
            return subprocess.run(
                [str(INSTALL_SH)],
                cwd=REPO_ROOT,
                env=env,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )

    def test_latest_without_matching_tag_prefix_fails_at_version_resolution(
        self,
    ) -> None:
        result = self.run_installer_with_release_list(
            textwrap.dedent(
                """\
                [
                  {
                    "tag_name": "v0.0.8",
                    "draft": false,
                    "prerelease": false
                  }
                ]
                """
            )
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn(
            "Failed to resolve the latest Open Interpreter release version.",
            result.stderr,
        )

    def test_latest_uses_matching_non_prerelease_tag_prefix(self) -> None:
        result = self.run_installer_with_release_list(
            textwrap.dedent(
                """\
                [
                  {
                    "tag_name": "v0.0.8",
                    "draft": false,
                    "prerelease": false
                  },
                  {
                    "tag_name": "rust-v0.3.0-beta.1",
                    "draft": false,
                    "prerelease": true
                  },
                  {
                    "tag_name": "rust-v0.2.0",
                    "draft": false,
                    "prerelease": false
                  }
                ]
                """
            )
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn(
            "Could not find Open Interpreter release assets for 0.2.0.", result.stderr
        )

    def test_default_codex_latest_skips_prereleases(self) -> None:
        result = self.run_installer_with_release_list(
            textwrap.dedent(
                """\
                [
                  {
                    "tag_name": "rust-v0.136.0-alpha.2",
                    "draft": false,
                    "prerelease": true
                  },
                  {
                    "tag_name": "rust-v0.135.0",
                    "draft": false,
                    "prerelease": false
                  }
                ]
                """
            ),
            repo="openai/codex",
            product_name="Codex CLI",
            package_asset_stem="codex-package",
            command_name="codex",
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn(
            "Could not find Codex CLI release assets for 0.135.0.", result.stderr
        )

    def build_package_release_fixture(self, tmp: Path) -> tuple[Path, str]:
        """Build a fake Open Interpreter package release plus a curl stub
        under tmp. Returns (fake_bin, target); fake_bin must be prepended to
        PATH so the installer downloads the fixture release."""
        target = current_installer_target()
        package_asset = f"open-interpreter-package-{target}.tar.gz"
        checksum_asset = "codex-package_SHA256SUMS"

        package_root = tmp / "package"
        (package_root / "bin").mkdir(parents=True)
        (package_root / "codex-path").mkdir()
        (package_root / "codex-resources").mkdir()

        write_executable(
            package_root / "bin" / "interpreter",
            "#!/bin/sh\nprintf 'interpreter 0.2.0\\n'\n",
        )
        write_executable(
            package_root / "bin" / "codex",
            "#!/bin/sh\nprintf 'codex 0.2.0\\n'\n",
        )
        write_executable(
            package_root / "bin" / "codex-code-mode-host",
            "#!/bin/sh\nexit 0\n",
        )
        write_executable(package_root / "codex-path" / "rg", "#!/bin/sh\nexit 0\n")
        if "linux" in target:
            write_executable(
                package_root / "codex-resources" / "bwrap", "#!/bin/sh\nexit 0\n"
            )

        (package_root / "codex-package.json").write_text(
            json.dumps(
                {
                    "layoutVersion": 1,
                    "version": "0.2.0",
                    "target": target,
                    "variant": "open-interpreter",
                    "entrypoint": "bin/interpreter",
                    "managedCodex": "bin/codex",
                    "resourcesDir": "codex-resources",
                    "pathDir": "codex-path",
                }
            ),
            encoding="utf-8",
        )

        archive = tmp / package_asset
        with tarfile.open(archive, "w:gz") as tar:
            for path in package_root.rglob("*"):
                tar.add(path, arcname=path.relative_to(package_root))
        package_digest = sha256(archive)

        checksum_file = tmp / checksum_asset
        checksum_file.write_text(
            f"{package_digest}  {package_asset}\n", encoding="utf-8"
        )
        checksum_digest = sha256(checksum_file)

        release_json = tmp / "release.json"
        release_json.write_text(
            json.dumps(
                {
                    "assets": [
                        {
                            "name": package_asset,
                            "digest": f"sha256:{package_digest}",
                        },
                        {
                            "name": checksum_asset,
                            "digest": f"sha256:{checksum_digest}",
                        },
                    ]
                },
                indent=2,
            ),
            encoding="utf-8",
        )

        fake_bin = tmp / "fake-bin"
        fake_bin.mkdir()
        fake_curl = fake_bin / "curl"
        fake_curl.write_text(
            textwrap.dedent(
                f"""\
                #!/bin/sh
                output=""
                url=""
                want_output=false
                for arg in "$@"; do
                  if [ "$want_output" = true ]; then
                    output="$arg"
                    want_output=false
                    continue
                  fi
                  case "$arg" in
                    -o)
                      want_output=true
                      ;;
                    http*)
                      url="$arg"
                      ;;
                  esac
                done

                emit() {{
                  if [ -n "$output" ]; then
                    cat "$1" > "$output"
                  else
                    cat "$1"
                  fi
                }}

                case "$url" in
                  */releases/tags/rust-v0.2.0)
                    emit {quote(str(release_json))}
                    ;;
                  */{package_asset})
                    emit {quote(str(archive))}
                    ;;
                  */{checksum_asset})
                    emit {quote(str(checksum_file))}
                    ;;
                  *)
                    echo "unexpected curl URL: $url" >&2
                    exit 42
                    ;;
                esac
                """
            ),
            encoding="utf-8",
        )
        fake_curl.chmod(fake_curl.stat().st_mode | stat.S_IXUSR)

        return fake_bin, target

    def run_package_installer(
        self, fake_bin: Path, codex_home: Path, install_dir: Path
    ) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [str(INSTALL_SH), "--release", "0.2.0"],
            cwd=REPO_ROOT,
            env={
                **os.environ,
                "PATH": f"{fake_bin}{os.pathsep}{os.environ['PATH']}",
                "CODEX_GITHUB_REPO": "openinterpreter/openinterpreter",
                "CODEX_INSTALL_PRODUCT_NAME": "Open Interpreter",
                "CODEX_PACKAGE_ASSET_STEM": "open-interpreter-package",
                "CODEX_COMMAND_NAME": "interpreter",
                "CODEX_ALIAS_COMMAND_NAMES": "i",
                "CODEX_RELEASE_TAG_PREFIX": "rust-v",
                "CODEX_NON_INTERACTIVE": "1",
                "CODEX_HOME": str(codex_home),
                "CODEX_INSTALL_DIR": str(install_dir),
            },
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )

    def test_open_interpreter_package_install_uses_metadata_entrypoint(self) -> None:
        with tempfile.TemporaryDirectory() as tmpdir:
            tmp = Path(tmpdir)
            fake_bin, _target = self.build_package_release_fixture(tmp)

            install_dir = tmp / "install-bin"
            codex_home = tmp / "home"
            result = self.run_package_installer(fake_bin, codex_home, install_dir)

            self.assertEqual(result.returncode, 0, result.stderr)
            current = codex_home / "packages" / "standalone" / "current"
            self.assertTrue((current / "codex-package.json").is_file())
            self.assertTrue((current / "bin" / "interpreter").is_file())
            self.assertTrue((current / "bin" / "codex").is_file())
            self.assertFalse((current / "interpreter").exists())
            self.assertFalse((current / "codex").exists())
            self.assertEqual(
                (install_dir / "interpreter").resolve(),
                (current / "bin" / "interpreter").resolve(),
            )
            self.assertEqual(
                (install_dir / "i").resolve(),
                (current / "bin" / "interpreter").resolve(),
            )
            if platform.system() == "Darwin":
                self.assertEqual(
                    (install_dir / "codex-code-mode-host").resolve(),
                    (current / "bin" / "codex-code-mode-host").resolve(),
                )

    def test_install_dir_matching_managed_executable_dir_is_rejected(self) -> None:
        # CODEX_INSTALL_DIR pointed at the release's own managed bin/
        # directory used to make the installer overwrite the real managed
        # executable with a symlink to itself (issue #1813). It must now be
        # rejected before any visible-link replacement happens.
        with tempfile.TemporaryDirectory() as tmpdir:
            tmp = Path(tmpdir)
            fake_bin, _target = self.build_package_release_fixture(tmp)

            codex_home = tmp / "home"
            install_dir = codex_home / "packages" / "standalone" / "current" / "bin"
            result = self.run_package_installer(fake_bin, codex_home, install_dir)

            self.assertNotEqual(result.returncode, 0)
            self.assertIn(
                "resolves to this release's managed executable directory",
                result.stderr,
            )

            current = codex_home / "packages" / "standalone" / "current"
            managed_interpreter = current / "bin" / "interpreter"
            self.assertTrue(managed_interpreter.is_file())
            self.assertFalse(managed_interpreter.is_symlink())


if __name__ == "__main__":
    unittest.main()
