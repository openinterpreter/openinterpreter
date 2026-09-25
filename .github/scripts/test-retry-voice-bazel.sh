#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "$0")" && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
export VOICE_BAZEL_RETRY_DELAY_SEC=0

cat > "$tmp/transient.sh" <<'EOF'
#!/usr/bin/env bash
count=$(cat "$1" 2>/dev/null || echo 0)
echo $((count + 1)) > "$1"
if (( count == 0 )); then
  echo 'Error downloading [https://www.nuget.org/api/v2/package/Microsoft.Windows.SDK.CPP/10.0.26100.7705]: Premature EOF'
  exit 1
fi
EOF
bash "$root/retry-voice-bazel.sh" bash "$tmp/transient.sh" "$tmp/transient.count" > "$tmp/transient.log"
[[ $(cat "$tmp/transient.count") == 2 ]]

cat > "$tmp/other.sh" <<'EOF'
#!/usr/bin/env bash
count=$(cat "$1" 2>/dev/null || echo 0)
echo $((count + 1)) > "$1"
echo 'Unrelated Bazel failure'
exit 1
EOF
if bash "$root/retry-voice-bazel.sh" bash "$tmp/other.sh" "$tmp/other.count" > "$tmp/other.log"; then
  echo 'Unrelated failure should not be retried or accepted' >&2
  exit 1
fi
[[ $(cat "$tmp/other.count") == 1 ]]
