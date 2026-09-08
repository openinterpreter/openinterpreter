#!/usr/bin/env python3

import hashlib
import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "README.md"
TRANSLATIONS = (ROOT / "README_ES.md", ROOT / "README_ZH.md", ROOT / "README_JA.md")
SOURCE_PATTERN = re.compile(
    r"<!-- README translation source: README\.md sha256=([0-9a-f]{64}) -->"
)


def main() -> int:
    expected = hashlib.sha256(SOURCE.read_bytes()).hexdigest()
    errors: list[str] = []

    for translation in TRANSLATIONS:
        match = SOURCE_PATTERN.search(translation.read_text(encoding="utf-8"))
        if match is None:
            errors.append(f"{translation.name}: missing README source checksum")
        elif match.group(1) != expected:
            errors.append(
                f"{translation.name}: translation is based on {match.group(1)}, "
                f"but README.md is {expected}"
            )

    if errors:
        print("README translations are out of date:", file=sys.stderr)
        for error in errors:
            print(f"- {error}", file=sys.stderr)
        print(
            "Update every translated README, then replace its source checksum "
            "with the SHA-256 of README.md.",
            file=sys.stderr,
        )
        return 1

    print(f"README translations match source checksum {expected}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
