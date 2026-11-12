"""Print CODEX_COMPAT_RELEASES JSON for the existing Bazel compatibility tests.

For local runs, export CODEX_COMPAT_RELEASES="$(python3 <path-to-this-script>)"
before invoking Bazel. CI resolves it once before building the test targets.
"""

import json
import re
import sys
from datetime import datetime, timedelta, timezone

from contract_releases import PACKAGE, github_read, list_releases, select_stables

STABLE_LOOKBACK_DAYS = 60


def main() -> None:
    target_date = datetime.now(timezone.utc) - timedelta(days=STABLE_LOOKBACK_DAYS)
    tags = select_stables(list_releases(exclude_prereleases=True), target_date)
    releases = {}
    for tag in tags:
        metadata = json.loads(
            github_read(["gh", "api", f"repos/openai/codex/releases/tags/{tag}"])
        )
        asset = next(asset for asset in metadata["assets"] if asset["name"] == PACKAGE)
        digest = asset.get("digest") or ""
        if not re.fullmatch(r"sha256:[0-9a-f]{64}", digest):
            raise ValueError(f"Missing SHA-256 digest for {tag}/{PACKAGE}")
        releases[tag] = {"version": tag.removeprefix("rust-v"), "sha256": digest[7:]}
    print(
        f"Compatibility releases: latest={tags[0]}, "
        f"closest to {target_date.isoformat()}={tags[-1]}",
        file=sys.stderr,
    )
    print(json.dumps({"latest": releases[tags[0]], "historical": releases[tags[-1]]}))


if __name__ == "__main__":
    main()
