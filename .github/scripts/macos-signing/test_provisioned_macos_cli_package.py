"""Exercise profile validation, relocation and signing without release credentials."""

import copy
import hashlib
import json
import plistlib
import shutil
import ssl
import subprocess
import tarfile
import tempfile
import unittest
from dataclasses import replace
from datetime import datetime, timedelta, timezone
from pathlib import Path

import provisioned_macos_cli_package as bundle


class ProvisioningTestCase(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        directory = tempfile.TemporaryDirectory()
        cls.addClassCleanup(directory.cleanup)
        root = Path(directory.name)
        key = root / "test-key.pem"
        certificate = root / "test-certificate.pem"
        subprocess.run(
            [
                "openssl",
                "req",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-keyout",
                str(key),
                "-out",
                str(certificate),
                "-days",
                "1",
                "-subj",
                "/CN=CLI provisioning test/OU=TESTTEAM01",
            ],
            check=True,
            capture_output=True,
        )
        cert_der = ssl.PEM_cert_to_DER_cert(certificate.read_text())
        now = datetime.now(timezone.utc).replace(tzinfo=None, microsecond=0)
        cls.profile = {
            "TeamIdentifier": ["TESTTEAM01"],
            "ApplicationIdentifierPrefix": ["TESTTEAM01"],
            "Entitlements": {
                "com.apple.application-identifier": "TESTTEAM01.com.openai.codex.cli",
                "com.apple.developer.team-identifier": "TESTTEAM01",
                "keychain-access-groups": ["TESTTEAM01.*"],
            },
            "CreationDate": now - timedelta(days=1),
            "ExpirationDate": now + timedelta(days=1),
            "ProvisionsAllDevices": True,
            "DeveloperCertificates": [cert_der],
        }
        payload = root / "profile.plist"
        payload.write_bytes(plistlib.dumps(cls.profile))
        profile_path = root / "test.provisionprofile"
        subprocess.run(
            [
                "openssl",
                "cms",
                "-sign",
                "-binary",
                "-nodetach",
                "-outform",
                "DER",
                "-in",
                str(payload),
                "-signer",
                str(certificate),
                "-inkey",
                str(key),
                "-out",
                str(profile_path),
            ],
            check=True,
            capture_output=True,
        )
        cls.configuration = bundle.ProfileConfiguration(
            profile=profile_path,
            profile_sha256=hashlib.sha256(profile_path.read_bytes()).hexdigest(),
            certificate_sha256=hashlib.sha256(cert_der).hexdigest(),
            team_id="TESTTEAM01",
        )


class ProvisionedCliTests(ProvisioningTestCase):
    def test_profile_configuration_fails_closed(self):
        self.assertEqual(bundle.load_profile(self.configuration), self.profile)
        for changes in (
            {"profile_sha256": "0" * 64},
            {"certificate_sha256": "0" * 64},
            {"team_id": "OTHERTEAM1"},
            {"profile_sha256": ""},
            {"certificate_sha256": ""},
            {"team_id": ""},
        ):
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                bundle.load_profile(replace(self.configuration, **changes))

    def test_rejects_profile_with_wrong_identity_certificate_or_validity(self):
        for field, value in (
            ("TeamIdentifier", ["OTHERTEAM"]),
            ("DeveloperCertificates", [b"another certificate"]),
            (
                "ExpirationDate",
                datetime(2000, 1, 1, tzinfo=timezone.utc).replace(tzinfo=None),
            ),
            (
                "CreationDate",
                datetime(2099, 1, 1, tzinfo=timezone.utc).replace(tzinfo=None),
            ),
            ("ProvisionsAllDevices", False),
            ("ProvisionedDevices", ["device"]),
        ):
            with self.subTest(field=field):
                profile = copy.deepcopy(self.profile)
                profile[field] = value
                with self.assertRaises(ValueError):
                    bundle.validate_profile(profile, self.configuration)
        for field, value in (
            (
                "com.apple.application-identifier",
                f"{self.configuration.team_id}.another.app",
            ),
            ("keychain-access-groups", [f"{self.configuration.team_id}.another.app"]),
            ("get-task-allow", True),
        ):
            with self.subTest(field=field):
                profile = copy.deepcopy(self.profile)
                profile["Entitlements"][field] = value
                with self.assertRaises(ValueError):
                    bundle.validate_profile(profile, self.configuration)

    def test_archive_relocation_copy_and_symlink_preserve_launcher(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            package = root / "original package"
            metadata = {
                "variant": "codex",
                "layoutVersion": 1,
                "target": "aarch64-apple-darwin",
                "entrypoint": "bin/codex",
            }
            for relative in ("bin/codex", *bundle.HELPERS):
                path = package / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text('#!/bin/sh\nprintf "%s\\n" "$@"\nexit 23\n')
                path.chmod(0o755)
            (package / "codex-package.json").write_text(json.dumps(metadata))
            original_binary = (package / "bin/codex").read_bytes()
            bundle.prepare(package, root / "reports", self.configuration)
            with self.assertRaises(FileExistsError):
                bundle.prepare(package, root / "reports", self.configuration)
            with self.assertRaisesRegex(ValueError, "Package target differs"):
                bundle.verify(
                    package, root / "reports", "x86_64-apple-darwin", self.configuration
                )
            self.assertEqual(
                (package / bundle.EXECUTABLE).read_bytes(), original_binary
            )
            self.assertEqual(
                json.loads((package / "codex-package.json").read_text()), metadata
            )

            archive_path = root / "package.tar.gz"
            with tarfile.open(archive_path, "w:gz") as archive:
                archive.add(package, arcname=".")
            relocated = root / "relocated package"
            with tarfile.open(archive_path) as archive:
                archive.extractall(relocated, filter="data")
            shutil.rmtree(package)
            # npm currently dereferences symlinks when copying packages. The
            # launcher remains a script and the provisioned executable stays put.
            copied = root / "copied package"
            shutil.copytree(relocated, copied)
            link = root / "installed codex"
            link.symlink_to(Path("copied package/bin/codex"))
            absolute_link = root / "absolute codex"
            absolute_link.symlink_to(link)
            for entry in (
                relocated / "bin/codex",
                copied / "bin/codex",
                link,
                absolute_link,
            ):
                with self.subTest(entry=entry):
                    result = subprocess.run(
                        [str(entry), "space argument", "*", "--flag"],
                        cwd="/",
                        capture_output=True,
                        text=True,
                        check=False,
                    )
                    self.assertEqual(
                        (result.returncode, result.stdout, result.stderr),
                        (23, "space argument\n*\n--flag\n", ""),
                    )
            (copied / bundle.APP / "Contents/embedded.provisionprofile").write_bytes(
                b"tampered"
            )
            with self.assertRaisesRegex(ValueError, "Embedded profile differs"):
                bundle.verify(
                    copied, root / "reports", "aarch64-apple-darwin", self.configuration
                )


if __name__ == "__main__":
    unittest.main()
