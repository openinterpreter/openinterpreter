#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
log="$(mktemp)"
trap 'rm -f "${log}"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

# A repository/patch fetch failure and a genuinely stale lockfile share this
# step. Keep Bazel's exact fail-closed check, but make its diagnostic bounded
# so a large dependency log cannot hide the actionable error.
if "${repo_root}/.github/scripts/run_bazel_with_buildbuddy.py" mod deps --lockfile_mode=error >"${log}" 2>&1; then
  echo 'MODULE.bazel.lock verified'
else
  status=$?
  python3 - "${log}" "${status}" <<'PY'
import json
import re
import sys
from pathlib import Path

log_path = Path(sys.argv[1])
log = log_path.read_text(encoding="utf-8", errors="replace")
status = int(sys.argv[2])
errors = [line.strip() for line in log.splitlines() if re.match(r"^(?:ERROR:|Error in fail:|error:)", line.strip())]
patterns = (
    ("patch_failure", r"patch failed|failed to apply patch|hunk \d+ failed|can't find file to patch"),
    ("stale_lock", r"(?:module\.bazel\.lock|lockfile).*(?:out.of.date|no longer up.to.date|stale|needs? to be updated)|(?:out.of.date|no longer up.to.date|stale).*(?:module\.bazel\.lock|lockfile)"),
    ("extension_failure", r"error evaluating module extension|duplicate crate\.annotation"),
    ("fetch_failure", r"failed to fetch repository|error downloading|fetching repository.*failed"),
)
diagnostic = "\n".join(errors).lower()
failure_class = next((name for name, pattern in patterns if re.search(pattern, diagnostic)), "other_bazel_failure")
safe = []
for line in errors[:4]:
    line = re.sub(r"https?://\S+", "[url]", line)
    line = re.sub(r"(?i)(?:token|password|secret|authorization)\s*[:=]\s*\S+", "[credential]", line)
    safe.append(line[:300])
print(json.dumps({
    "check": "module_bazel_lock",
    "result": "failure",
    "class": failure_class,
    "bazel_exit_code": status,
    "log_bytes": log_path.stat().st_size,
    "error_lines_total": len(errors),
    "errors": safe,
    "truncated": len(errors) > len(safe),
}, sort_keys=True))
PY
  exit "${status}"
fi
