#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/build-interpreter-release.sh [options]

Options:
  --target <target>        Rust target triple (defaults to host platform)
  --install-dir <dir>      Visible bin directory for shims
  --home <dir>             Open Interpreter home directory
  -j, --jobs <N>           Number of parallel Cargo build jobs (default: 1)
  -h, --help               Show this help message

Builds a local Open Interpreter standalone package using the same package
layout as the public installer, stages it under INTERPRETER_HOME, and
installs interpreter/i shims into the visible bin directory.
EOF
}

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
codex_rs_dir="$repo_root/codex-rs"
target=""
install_dir="${OPEN_INTERPRETER_INSTALL_DIR:-${CODEX_INSTALL_DIR:-$HOME/.local/bin}}"
interpreter_home="${INTERPRETER_HOME:-$HOME/.openinterpreter}"
build_jobs="${CARGO_BUILD_JOBS:-1}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --target=*)
      target="${1#*=}"
      shift
      ;;
    --target)
      target="${2:?--target requires a value}"
      shift 2
      ;;
    --install-dir=*)
      install_dir="${1#*=}"
      shift
      ;;
    --install-dir)
      install_dir="${2:?--install-dir requires a value}"
      shift 2
      ;;
    --home=*)
      interpreter_home="${1#*=}"
      shift
      ;;
    --home)
      interpreter_home="${2:?--home requires a value}"
      shift 2
      ;;
    --jobs=*|-j=*)
      build_jobs="${1#*=}"
      shift
      ;;
    --jobs|-j)
      build_jobs="${2:?--jobs requires a value}"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unexpected argument: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

if [[ ! "$build_jobs" =~ ^[1-9][0-9]*$ ]]; then
  echo "Cargo build jobs must be a positive integer: $build_jobs" >&2
  exit 1
fi

# Resolve paths to absolute locations so relative CLI arguments work safely.
mkdir -p "$install_dir" "$interpreter_home"
install_dir="$(cd "$install_dir" && pwd -P)"
interpreter_home="$(cd "$interpreter_home" && pwd -P)"

package_root="$interpreter_home/packages/standalone"
releases_dir="$package_root/releases"
current_link="$package_root/current"
target_label="host"
if [[ -n "$target" ]]; then
  target_label="$target"
fi

package_dir="$releases_dir/local-$target_label"
staging_dir="$releases_dir/.staging.local-$target_label.$$"
cleanup_staging=true

cleanup() {
  if [[ "${cleanup_staging:-false}" == "true" && -n "${staging_dir:-}" ]]; then
    rm -rf "$staging_dir"
  fi
}
trap cleanup EXIT INT TERM

replace_symlink() {
  local target_path="$1"
  local link_path="$2"
  local tmp_link="${link_path}.$$"

  if [[ -d "$link_path" && ! -L "$link_path" ]]; then
    echo "Refusing to replace non-symlink directory: $link_path" >&2
    exit 1
  fi

  mkdir -p "$(dirname -- "$link_path")"
  rm -f "$tmp_link"
  ln -s "$target_path" "$tmp_link"
  rm -f "$link_path"
  mv -f "$tmp_link" "$link_path"
}

echo "Building local Open Interpreter standalone package..."
echo "Workspace: $codex_rs_dir"
echo "Open Interpreter home: $interpreter_home"
echo "Visible bin directory: $install_dir"
echo "Cargo build jobs: $build_jobs"

mkdir -p "$releases_dir" "$install_dir"
rm -rf "$staging_dir"

python_command="${OPEN_INTERPRETER_PYTHON:-}"
if [[ -n "$python_command" ]]; then
  if ! command -v "$python_command" >/dev/null 2>&1 ||
     ! "$python_command" -c 'import tomllib' >/dev/null 2>&1; then
    echo "OPEN_INTERPRETER_PYTHON must name a Python interpreter with tomllib support" >&2
    exit 1
  fi
else
  for candidate in python3 python3.13 python3.12 python3.11; do
    if command -v "$candidate" >/dev/null 2>&1 &&
       "$candidate" -c 'import tomllib' >/dev/null 2>&1; then
      python_command="$candidate"
      break
    fi
  done
  if [[ -z "$python_command" ]]; then
    echo "Python 3.11 or newer is required to build the release package" >&2
    exit 1
  fi
fi

(
  cd "$repo_root"
  package_args=(
    --variant open-interpreter
    --cargo-profile release
    --package-dir "$staging_dir"
    --force
  )
  if [[ -n "$target" ]]; then
    package_args=(--target "$target" "${package_args[@]}")
  fi
  CODEX_REPO_ROOT="$repo_root" \
    CARGO_BUILD_JOBS="$build_jobs" \
    "$python_command" scripts/build_codex_package.py "${package_args[@]}"
)

if [[ -e "$package_dir" || -L "$package_dir" ]]; then
  rm -rf "$package_dir"
fi
mv "$staging_dir" "$package_dir"
cleanup_staging=false

replace_symlink "$package_dir" "$current_link"
replace_symlink "$current_link/bin/interpreter" "$install_dir/interpreter"
replace_symlink "$current_link/bin/interpreter" "$install_dir/i"

echo
echo "Built and staged:"
echo "  $package_dir/bin/interpreter"
echo
echo "Installed shims:"
echo "  $install_dir/interpreter -> $current_link/bin/interpreter"
echo "  $install_dir/i -> $current_link/bin/interpreter"
echo

if [[ -x "$install_dir/interpreter" ]]; then
  "$install_dir/interpreter" --version
else
  echo "Error: Installed binary at $install_dir/interpreter is not executable" >&2
  exit 1
fi
