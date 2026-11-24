"""Exercise how full Cargo CI consumes prepared native inputs."""

import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

from cargo_ci import environment, main, stage
from package_runtime import runtime_files
from runtime import PLUGINS, digest, required_library_paths
from sdk import MODULES


def write_files(root, contents):
    records = []
    for name, data in contents.items():
        path = root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        path.chmod(0o444)  # Bazel materializes these inputs read-only.
        records.append({"path": name, "sha256": digest(path)})
    return records


class CargoCiTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory(prefix="voice cargo ")
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name).resolve()
        self.sdk = self.root / "sdk"
        self.runtime = self.root / "runtime"
        self.output = self.root / "stage"
        self.commit = "a" * 40

    def inputs(self, target, alias, plugin):
        canonical = required_library_paths(target)[0]
        plugins = [plugin.format(name) for name in PLUGINS]
        metadata = (
            "prefix=/producer\nName: {name}\nDescription: fixture\nVersion: 1.28\n"
        )
        sdk = {alias: b"original", "lib/glib-2.0/include/glibconfig.h": b"header"}
        for name in MODULES:
            sdk[f"lib/pkgconfig/{name}.pc"] = metadata.format(name=name).encode()
        if not target.endswith("msvc"):
            sdk[canonical] = sdk[alias]
        sdk_files = write_files(self.sdk, sdk)
        libraries = write_files(
            self.runtime, {name: b"prepared" for name in [canonical, *plugins]}
        )
        libraries[0]["sourceSha256"] = digest(self.sdk / alias)
        identity = dict(
            schemaVersion=1,
            target=target,
            sourceCommit=self.commit,
            sourceManifestSha256=digest(Path(__file__).with_name("sources.json")),
        )
        (self.sdk / "sdk.json").write_text(json.dumps(dict(identity, files=sdk_files)))
        receipt = dict(
            identity, developmentOnly=True, libraries=libraries, plugins=plugins
        )
        (self.runtime / "runtime.json").write_text(json.dumps(receipt))
        return canonical

    def test_relocated_metadata_runtime_and_cargo_probe(self):
        target = "aarch64-apple-darwin"
        alias = "lib/libgio-2.0.dylib"
        canonical = self.inputs(target, alias, "plugins/libgst{}.dylib")
        stage(self.sdk, self.runtime, self.output, target, self.commit)
        moved = self.root / "moved native inputs"
        self.output.rename(moved)
        native = moved / "runtime"
        self.assertEqual(
            (native / alias).read_bytes(), (self.runtime / canonical).read_bytes()
        )
        self.assertEqual(
            runtime_files(native, target), runtime_files(self.runtime, target)
        )

        host = (
            os.environ.get("VOICE_PKG_CONFIG")
            or shutil.which("pkg-config")
            or shutil.which("pkgconf")
        )
        if not host:
            self.skipTest("host pkg-config is required to probe relocated metadata")
        pkgdir = moved / "sdk/lib/pkgconfig"
        probe_env = dict(os.environ, PKG_CONFIG_PATH="", PKG_CONFIG_LIBDIR=str(pkgdir))
        result = subprocess.check_output(
            [host, "--variable=prefix", "gio-2.0"], env=probe_env, text=True
        )
        # pkgconf can escape spaces while pkg-config can print the raw path.
        raw = result.strip()
        words = shlex.split(raw)
        self.assertEqual(
            Path(words[0] if len(words) == 1 else raw).resolve(), moved / "sdk"
        )

        env_file = self.root / "github env"
        ci_env = dict(
            GITHUB_SHA=self.commit, GITHUB_ENV=str(env_file), VOICE_PKG_CONFIG=host
        )
        argv = ["cargo_ci", "configure", "--root", str(moved), "--target", target]
        with patch.dict(os.environ, ci_env), patch.object(sys, "argv", argv):
            main()
            written = env_file.read_text()
            values = dict(line.split("=", 1) for line in written.splitlines())
            self.assertEqual(
                values["PKG_CONFIG_PATH_aarch64_apple_darwin"], str(pkgdir)
            )
            self.assertNotIn("PKG_CONFIG_LIBDIR_aarch64_apple_darwin", values)
            (pkgdir / "gio-2.0.pc").unlink()
            with self.assertRaises(subprocess.CalledProcessError):
                main()
            self.assertEqual(env_file.read_text(), written)

    def test_arm64_import_library_and_dll_with_x64_host_pkgconf(self):
        target = "aarch64-pc-windows-msvc"
        alias = "lib/gio-2.0.lib"
        canonical = self.inputs(target, alias, "bin/gst{}.dll")
        stage(self.sdk, self.runtime, self.output, target, self.commit)
        native = self.output / "runtime"
        self.assertEqual(
            (native / "bin/gio-2.0.lib").read_bytes(), (self.sdk / alias).read_bytes()
        )
        self.assertEqual(
            (native / canonical).read_bytes(), (self.runtime / canonical).read_bytes()
        )
        tool = self.root / "x64 pkgconf.exe"
        tool.touch()
        with patch.dict(os.environ, GITHUB_SHA=self.commit):
            values = environment(self.output, target, tool)
        self.assertEqual(values["PKG_CONFIG_aarch64_pc_windows_msvc"], str(tool))
        self.assertEqual(
            values["SYSTEM_DEPS_GSTREAMER_1_0_SEARCH_NATIVE"], str(native / "bin")
        )

    def test_mismatched_receipt_and_corrupt_sdk_fail_staging(self):
        target = "x86_64-unknown-linux-gnu"
        alias = "lib/libgio-2.0.so"
        self.inputs(target, alias, "lib/gstreamer-1.0/libgst{}.so")
        for triple, commit in [
            (target, "b" * 40),
            ("aarch64-unknown-linux-gnu", self.commit),
        ]:
            with self.assertRaisesRegex(ValueError, "do not match"):
                stage(self.sdk, self.runtime, self.output, triple, commit)
        source = self.sdk / alias
        source.chmod(0o644)
        source.write_bytes(b"corrupt")
        with self.assertRaisesRegex(ValueError, "receipt"):
            stage(self.sdk, self.runtime, self.output, target, self.commit)
        self.assertFalse(self.output.exists())
