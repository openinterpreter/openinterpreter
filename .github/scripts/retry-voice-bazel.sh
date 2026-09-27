#!/usr/bin/env bash
set -euo pipefail

for attempt in 1 2 3; do
  log=$(mktemp)
  if "$@" 2>&1 | tee "$log"; then
    rm -f "$log"
    exit 0
  fi
  if (( attempt == 3 )) || ! grep -Eq 'Error downloading .*Microsoft\.Windows\.SDK\.CPP.*Premature EOF' "$log"; then
    rm -f "$log"
    exit 1
  fi
  rm -f "$log"
  echo "Pinned Windows SDK download ended early; retrying Bazel ($attempt/3)" >&2
  sleep "${VOICE_BAZEL_RETRY_DELAY_SEC:-10}"
done
