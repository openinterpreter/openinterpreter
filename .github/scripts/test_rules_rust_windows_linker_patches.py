"""Keep both Windows linker patches active without broadening linker fallback."""

from pathlib import Path
import re
import unittest


ROOT = Path(__file__).resolve().parents[2]


class RulesRustWindowsLinkerPatchesTest(unittest.TestCase):
    def test_declared_linker_follows_runtime_artifact_patch(self) -> None:
        module = (ROOT / "MODULE.bazel").read_text(encoding="utf-8")
        block = module.split('rules_rust.patch(', 1)[1].split('use_repo(rules_rust', 1)[0]
        patches = re.findall(r'"//patches:([a-z_]+\.patch)"', block)
        self.assertLess(
            patches.index('rules_rust_windows_msvc_direct_link_args.patch'),
            patches.index('rules_rust_declared_msvc_linker.patch'),
        )

    def test_declared_linker_fails_closed_on_toolchain_files(self) -> None:
        patch = (
            ROOT / 'patches/rules_rust_declared_msvc_linker.patch'
        ).read_text(encoding='utf-8')
        for required in (
            'for_rustc and not use_rust_linker',
            'toolchain.target_abi == "msvc"',
            'paths.basename(cc_linker).lower() in ("clang-cl", "clang-cl.exe")',
            'if file.path == linker_path:',
            'if not msvc_lld_link:',
            'is_direct_driver = bool(msvc_lld_link)',
        ):
            with self.subTest(required=required):
                self.assertIn(required, patch)


if __name__ == '__main__':
    unittest.main()
