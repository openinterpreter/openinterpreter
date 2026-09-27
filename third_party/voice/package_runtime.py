"""Validate prepared runtime files before copying them into a private package.

The runtime receipt records preparation, not authenticity. Preserve its bytes and
layout; native loader inspection remains the platform preparer's responsibility.
"""

import hashlib
import json
from pathlib import Path
import re

from runtime import PLUGINS, digest, required_library_paths


def runtime_files(
    root: Path, target: str, *, public_release: bool = False
) -> dict[str, str]:
    manifest_path = root / "runtime.json"
    if manifest_path.is_symlink() or not manifest_path.is_file():
        raise ValueError("runtime manifest must be a regular file")
    with manifest_path.open("rb") as source:
        data = source.read(1024 * 1024 + 1)
    if len(data) > 1024 * 1024:
        raise ValueError("runtime manifest exceeds limit")
    manifest = json.loads(data)
    if (
        manifest.get("schemaVersion") != 1
        or manifest.get("developmentOnly") is not (not public_release)
        or (public_release and manifest.get("distribution") != "publicRelease")
        or manifest.get("target") != target
        or manifest.get("sourceManifestSha256")
        != digest(Path(__file__).with_name("sources.json"))
        or not re.fullmatch(r"[0-9a-f]{40}", manifest.get("sourceCommit", ""))
    ):
        raise ValueError(
            "runtime receipt does not match the source inputs and helper target"
        )
    if target.endswith("-apple-darwin"):
        pattern, plugin = (
            r"(?:lib|plugins)/[A-Za-z0-9_+.-]+\.dylib",
            "plugins/libgst{}.dylib",
        )
    elif target.endswith("-unknown-linux-gnu"):
        pattern, plugin = (
            r"lib/(?:gstreamer-1\.0/)?[A-Za-z0-9_+.-]+\.so(?:\.[0-9]+)*",
            "lib/gstreamer-1.0/libgst{}.so",
        )
    elif target.endswith("-pc-windows-msvc"):
        pattern, plugin = r"bin/[A-Za-z0-9_+.-]+\.[dD][lL][lL]", "bin/gst{}.dll"
    else:
        raise ValueError("unsupported native runtime target")
    libraries = manifest.get("libraries", [])
    if not 1 <= len(libraries) <= 128:
        raise ValueError("unexpected runtime inventory size")
    files, names = {}, set()
    for record in libraries:
        name, expected = record["path"], record["sha256"]
        if not re.fullmatch(pattern, name) or name.casefold() in names:
            raise ValueError("invalid or colliding runtime path")
        path = root / name
        if (
            path.parent.is_symlink()
            or path.is_symlink()
            or not path.is_file()
            or not path.resolve().is_relative_to(root)
        ):
            raise ValueError("runtime entries must be regular files inside the input")
        if not re.fullmatch(r"[0-9a-f]{64}", expected) or digest(path) != expected:
            raise ValueError("runtime file digest mismatch")
        names.add(name.casefold())
        files[name] = expected
    plugins = sorted(plugin.format(name) for name in PLUGINS)
    if sorted(manifest.get("plugins", [])) != plugins or not set(plugins).issubset(
        files
    ):
        raise ValueError("runtime must include exactly the selected plugins")
    if not set(required_library_paths(target)).issubset(files):
        raise ValueError("runtime must include the required libraries")
    files["runtime.json"] = hashlib.sha256(data).hexdigest()
    return files
