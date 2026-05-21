#!/bin/sh
set -eu

if command -v sccache >/dev/null 2>&1; then
  exec sccache "$@"
fi

exec "$@"
