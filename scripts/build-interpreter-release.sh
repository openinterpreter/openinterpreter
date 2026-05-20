#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
codex_rs_dir="$repo_root/codex-rs"
target_dir="$codex_rs_dir/target/release"
shim_dir="${HOME}/.local/bin"
shim_path="$shim_dir/interpreter"
build_jobs="${CARGO_BUILD_JOBS:-1}"

echo "Building Open Interpreter release binaries..."
echo "Workspace: $codex_rs_dir"
echo "Cargo build jobs: $build_jobs"

cd "$codex_rs_dir"

cargo build \
  --jobs "$build_jobs" \
  --target-dir "$codex_rs_dir/target" \
  --release \
  -p codex-server-cli --bins \
  -p codex-root-tui --bin interpreter-root-tui \
  -p codex-exec --bin interpreter-exec

required_bins=(
  interpreter
  interpreter-tui
  interpreter-app-server
  interpreter-root-tui
  interpreter-exec
)

for bin in "${required_bins[@]}"; do
  bin_path="$target_dir/$bin"
  if [[ ! -x "$bin_path" ]]; then
    echo "Missing required executable: $bin_path" >&2
    exit 1
  fi
done

mkdir -p "$shim_dir"
cat >"$shim_path" <<EOF
#!/bin/sh
exec "$target_dir/interpreter" "\$@"
EOF
chmod +x "$shim_path"

echo
echo "Built and verified:"
for bin in "${required_bins[@]}"; do
  echo "  $target_dir/$bin"
done
echo
echo "Installed shim:"
echo "  $shim_path -> $target_dir/interpreter"
echo
"$shim_path" --version
