#!/bin/sh

set -eu

GITHUB_REPO="${OPEN_INTERPRETER_GITHUB_REPO:-KillianLucas/oix}"
RELEASE="latest"

BIN_DIR="${OPEN_INTERPRETER_INSTALL_DIR:-$HOME/.local/bin}"
BIN_PATH="$BIN_DIR/interpreter"
OPEN_INTERPRETER_HOME_DIR="${OPEN_INTERPRETER_HOME:-$HOME/.openinterpreter}"
STANDALONE_ROOT="$OPEN_INTERPRETER_HOME_DIR/packages/standalone"
RELEASES_DIR="$STANDALONE_ROOT/releases"
CURRENT_LINK="$STANDALONE_ROOT/current"
LOCK_FILE="$STANDALONE_ROOT/install.lock"
LOCK_DIR="$STANDALONE_ROOT/install.lock.d"
LOCK_STALE_AFTER_SECS=600

path_action="already"
path_profile=""
lock_kind=""
tmp_dir=""

step() {
  printf '==> %s\n' "$1"
}

warn() {
  printf 'WARNING: %s\n' "$1" >&2
}

github_token() {
  if [ -n "${GITHUB_TOKEN:-}" ]; then
    printf '%s\n' "$GITHUB_TOKEN"
    return
  fi

  if [ -n "${GH_TOKEN:-}" ]; then
    printf '%s\n' "$GH_TOKEN"
    return
  fi
}

normalize_version() {
  case "$1" in
    "" | latest)
      printf 'latest\n'
      ;;
    v*)
      printf '%s\n' "${1#v}"
      ;;
    *)
      printf '%s\n' "$1"
      ;;
  esac
}

parse_args() {
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --release)
        if [ "$#" -lt 2 ]; then
          echo "--release requires a value." >&2
          exit 1
        fi
        RELEASE="$2"
        shift
        ;;
      --repo)
        if [ "$#" -lt 2 ]; then
          echo "--repo requires owner/name." >&2
          exit 1
        fi
        GITHUB_REPO="$2"
        shift
        ;;
      --help | -h)
        cat <<EOF
Usage: install.sh [--release VERSION] [--repo OWNER/REPO]

Installs or updates Open Interpreter from GitHub Releases.
EOF
        exit 0
        ;;
      *)
        echo "Unknown argument: $1" >&2
        exit 1
        ;;
    esac
    shift
  done
}

download_file() {
  url="$1"
  output="$2"
  token="$(github_token || true)"
  accept_header="application/octet-stream"

  if command -v curl >/dev/null 2>&1; then
    if [ -n "$token" ]; then
      curl -fsSL -H "Authorization: Bearer $token" -H "Accept: $accept_header" -H "X-GitHub-Api-Version: 2022-11-28" "$url" -o "$output"
    else
      curl -fsSL -H "Accept: $accept_header" "$url" -o "$output"
    fi
    return
  fi

  if command -v wget >/dev/null 2>&1; then
    if [ -n "$token" ]; then
      wget -q --header="Authorization: Bearer $token" --header="Accept: $accept_header" --header="X-GitHub-Api-Version: 2022-11-28" -O "$output" "$url"
    else
      wget -q --header="Accept: $accept_header" -O "$output" "$url"
    fi
    return
  fi

  echo "curl or wget is required to install Open Interpreter." >&2
  exit 1
}

download_text() {
  url="$1"
  token="$(github_token || true)"
  accept_header="application/vnd.github+json"

  case "$url" in
    https://api.github.com/*/releases/assets/*)
      accept_header="application/octet-stream"
      ;;
  esac

  if command -v curl >/dev/null 2>&1; then
    if [ -n "$token" ]; then
      curl -fsSL -H "Authorization: Bearer $token" -H "Accept: $accept_header" -H "X-GitHub-Api-Version: 2022-11-28" "$url"
    else
      curl -fsSL -H "Accept: $accept_header" "$url"
    fi
    return
  fi

  if command -v wget >/dev/null 2>&1; then
    if [ -n "$token" ]; then
      wget -q --header="Authorization: Bearer $token" --header="Accept: $accept_header" --header="X-GitHub-Api-Version: 2022-11-28" -O - "$url"
    else
      wget -q --header="Accept: $accept_header" -O - "$url"
    fi
    return
  fi

  echo "curl or wget is required to install Open Interpreter." >&2
  exit 1
}

release_url_for_asset() {
  asset="$1"
  resolved_version="$2"

  printf 'https://github.com/%s/releases/download/v%s/%s\n' "$GITHUB_REPO" "$resolved_version" "$asset"
}

release_api_url_for_asset() {
  asset="$1"
  resolved_version="$2"
  release_json="$(download_text "$(release_metadata_url "$resolved_version")")"

  asset_url="$(printf '%s\n' "$release_json" | awk -v asset="$asset" '
    {
      if (/"url":[[:space:]]*"https:\/\/api[.]github[.]com\/repos\/[^"]+\/releases\/assets\/[0-9]+"/) {
        sub(/^.*"url":[[:space:]]*"/, "")
        sub(/".*$/, "")
        candidate_url = $0
      }

      if ($0 ~ "\"name\":[[:space:]]*\"" asset "\"" && candidate_url != "") {
        asset_url = candidate_url
      }
    }
    END {
      if (asset_url != "") {
        print asset_url
      }
    }
  ')"

  if [ -z "$asset_url" ]; then
    echo "Could not find release asset $asset in v$resolved_version." >&2
    exit 1
  fi

  printf '%s\n' "$asset_url"
}

release_metadata_url() {
  resolved_version="$1"

  printf 'https://api.github.com/repos/%s/releases/tags/v%s\n' "$GITHUB_REPO" "$resolved_version"
}

latest_release_metadata_url() {
  printf 'https://api.github.com/repos/%s/releases/latest\n' "$GITHUB_REPO"
}

all_releases_metadata_url() {
  printf 'https://api.github.com/repos/%s/releases\n' "$GITHUB_REPO"
}

release_asset_digest() {
  asset="$1"
  resolved_version="$2"
  release_json="$(download_text "$(release_metadata_url "$resolved_version")")"

  digest="$(printf '%s\n' "$release_json" | awk -v asset="$asset" '
    {
      if ($0 ~ "\"name\":[[:space:]]*\"" asset "\"") {
        in_asset = 1
        asset_depth = depth
      }

      if (in_asset && /"digest":[[:space:]]*"[^"]+"/) {
        sub(/^.*"digest":[[:space:]]*"/, "")
        sub(/".*$/, "")
        digest = $0
      }

      line = $0
      opens = gsub(/\{/, "{", line)
      closes = gsub(/\}/, "}", line)
      depth += opens - closes

      if (in_asset && depth < asset_depth) {
        in_asset = 0
      }
    }
    END {
      if (digest != "") {
        print digest
      }
    }
  ')"

  case "$digest" in
    sha256:????????????????????????????????????????????????????????????????)
      printf '%s\n' "${digest#sha256:}"
      return
      ;;
  esac

  checksum_url="$(release_api_url_for_asset "$asset.sha256" "$resolved_version")"
  checksum_text="$(download_text "$checksum_url")"
  checksum="$(printf '%s\n' "$checksum_text" | awk '{print $1}' | head -n 1)"
  case "$checksum" in
    ????????????????????????????????????????????????????????????????)
      printf '%s\n' "$checksum"
      ;;
    *)
      echo "Could not find SHA-256 digest for release asset $asset." >&2
      exit 1
      ;;
  esac
}

file_sha256() {
  path="$1"

  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$path" | awk '{print $1}'
    return
  fi

  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$path" | awk '{print $1}'
    return
  fi

  if command -v openssl >/dev/null 2>&1; then
    openssl dgst -sha256 "$path" | sed 's/^.*= //'
    return
  fi

  echo "sha256sum, shasum, or openssl is required to verify the Open Interpreter download." >&2
  exit 1
}

verify_archive_digest() {
  archive_path="$1"
  expected_digest="$2"
  actual_digest="$(file_sha256 "$archive_path")"

  if [ "$actual_digest" != "$expected_digest" ]; then
    echo "Downloaded Open Interpreter archive checksum did not match release metadata." >&2
    echo "expected: $expected_digest" >&2
    echo "actual:   $actual_digest" >&2
    exit 1
  fi
}

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "$1 is required to install Open Interpreter." >&2
    exit 1
  fi
}

resolve_version() {
  normalized_version="$(normalize_version "$RELEASE")"

  if [ "$normalized_version" != "latest" ]; then
    printf '%s\n' "$normalized_version"
    return
  fi

  release_json="$(download_text "$(latest_release_metadata_url)" 2>/dev/null || true)"
  if [ -z "$release_json" ]; then
    release_json="$(download_text "$(all_releases_metadata_url)")"
  fi
  resolved="$(printf '%s\n' "$release_json" | sed -n 's/.*"tag_name":[[:space:]]*"v\([^"]*\)".*/\1/p' | head -n 1)"

  if [ -z "$resolved" ]; then
    echo "Failed to resolve the latest Open Interpreter release version." >&2
    exit 1
  fi

  printf '%s\n' "$resolved"
}

pick_profile() {
  case "$os:${SHELL:-}" in
    darwin:*/zsh)
      printf '%s\n' "$HOME/.zprofile"
      ;;
    darwin:*/bash)
      printf '%s\n' "$HOME/.bash_profile"
      ;;
    linux:*/zsh)
      printf '%s\n' "$HOME/.zshrc"
      ;;
    linux:*/bash)
      printf '%s\n' "$HOME/.bashrc"
      ;;
    *)
      printf '%s\n' "$HOME/.profile"
      ;;
  esac
}

add_to_path() {
  path_action="already"
  path_profile=""

  case ":$PATH:" in
    *":$BIN_DIR:"*)
      return
      ;;
  esac

  profile="$(pick_profile)"
  path_profile="$profile"
  begin_marker="# >>> Open Interpreter installer >>>"
  end_marker="# <<< Open Interpreter installer <<<"
  path_line="export PATH=\"$BIN_DIR:\$PATH\""

  if [ -f "$profile" ] && grep -F "$begin_marker" "$profile" >/dev/null 2>&1; then
    if grep -F "$path_line" "$profile" >/dev/null 2>&1; then
      path_action="configured"
      return
    fi

    if grep -F "$end_marker" "$profile" >/dev/null 2>&1; then
      rewrite_path_block "$profile" "$begin_marker" "$end_marker" "$path_line"
      path_action="updated"
      return
    fi
  fi

  append_path_block "$profile" "$begin_marker" "$end_marker" "$path_line"
  path_action="added"
}

append_path_block() {
  profile="$1"
  begin_marker="$2"
  end_marker="$3"
  path_line="$4"

  {
    printf '\n%s\n' "$begin_marker"
    printf '%s\n' "$path_line"
    printf '%s\n' "$end_marker"
  } >>"$profile"
}

rewrite_path_block() {
  profile="$1"
  begin_marker="$2"
  end_marker="$3"
  path_line="$4"
  tmp_profile="$tmp_dir/profile.$$.tmp"

  awk -v begin="$begin_marker" -v end="$end_marker" -v line="$path_line" '
    BEGIN {
      in_block = 0
      replaced = 0
    }
    $0 == begin {
      if (!replaced) {
        print begin
        print line
        print end
        replaced = 1
      }
      in_block = 1
      next
    }
    in_block {
      if ($0 == end) {
        in_block = 0
      }
      next
    }
    {
      print
    }
    END {
      if (in_block != 0) {
        exit 1
      }
    }
  ' "$profile" >"$tmp_profile"
  mv "$tmp_profile" "$profile"
}

mkdir_lock_is_stale() {
  [ -d "$LOCK_DIR" ] || return 1

  pid="$(cat "$LOCK_DIR/pid" 2>/dev/null || true)"
  started_at="$(cat "$LOCK_DIR/started_at" 2>/dev/null || true)"
  now="$(date +%s 2>/dev/null || printf '0')"

  case "$started_at" in
    ''|*[!0-9]*)
      started_at=0
      ;;
  esac

  if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
    return 1
  fi

  if [ "$started_at" -eq 0 ] || [ "$now" -eq 0 ]; then
    return 0
  fi

  [ $((now - started_at)) -ge "$LOCK_STALE_AFTER_SECS" ]
}

acquire_install_lock() {
  mkdir -p "$STANDALONE_ROOT"

  if [ "$os" = "darwin" ] && command -v lockf >/dev/null 2>&1; then
    : >>"$LOCK_FILE"
    exec 9<>"$LOCK_FILE"
    lockf 9
    lock_kind="lockf"
    return
  fi

  if command -v flock >/dev/null 2>&1; then
    exec 9>"$LOCK_FILE"
    flock 9
    lock_kind="flock"
    return
  fi

  while ! mkdir "$LOCK_DIR" 2>/dev/null; do
    if mkdir_lock_is_stale; then
      warn "Removing stale installer lock at $LOCK_DIR"
      rm -rf "$LOCK_DIR"
      continue
    fi
    sleep 1
  done

  printf '%s\n' "$$" >"$LOCK_DIR/pid"
  date +%s >"$LOCK_DIR/started_at" 2>/dev/null || true
  lock_kind="mkdir"
}

release_install_lock() {
  if [ "$lock_kind" = "mkdir" ]; then
    rm -rf "$LOCK_DIR" 2>/dev/null || true
  elif [ "$lock_kind" = "flock" ] || [ "$lock_kind" = "lockf" ]; then
    exec 9>&- 2>/dev/null || true
  fi
  lock_kind=""
}

cleanup_stale_install_artifacts() {
  mkdir -p "$RELEASES_DIR" "$STANDALONE_ROOT"

  find "$RELEASES_DIR" -mindepth 1 -maxdepth 1 -name '.staging.*' -exec rm -rf {} +
  find "$STANDALONE_ROOT" -mindepth 1 -maxdepth 1 -name '.current.*' -exec rm -f {} +

  if [ -d "$BIN_DIR" ]; then
    find "$BIN_DIR" -mindepth 1 -maxdepth 1 -name '.interpreter.*' -exec rm -f {} +
  fi
}

replace_path_with_symlink() {
  link_path="$1"
  link_target="$2"
  tmp_link="$3"

  rm -f "$tmp_link"
  ln -s "$link_target" "$tmp_link"

  if mv -Tf "$tmp_link" "$link_path" 2>/dev/null; then
    return
  fi

  if mv -hf "$tmp_link" "$link_path" 2>/dev/null; then
    return
  fi

  rm -f "$link_path"
  mv -f "$tmp_link" "$link_path"
}

version_from_binary() {
  interpreter_path="$1"

  if [ ! -x "$interpreter_path" ]; then
    return 1
  fi

  "$interpreter_path" --version 2>/dev/null | sed -n 's/.* \([0-9][0-9A-Za-z.+-]*\)$/\1/p' | head -n 1
}

current_installed_version() {
  version="$(version_from_binary "$CURRENT_LINK/interpreter" || true)"
  if [ -n "$version" ]; then
    printf '%s\n' "$version"
    return 0
  fi

  return 0
}

prompt_yes_no() {
  prompt="$1"

  if ( : </dev/tty ) 2>/dev/null; then
    printf '%s [y/N] ' "$prompt" >/dev/tty
    if ! IFS= read -r answer </dev/tty; then
      return 1
    fi
  elif [ -t 0 ]; then
    printf '%s [y/N] ' "$prompt"
    if ! IFS= read -r answer; then
      return 1
    fi
  else
    return 1
  fi

  case "$answer" in
    y | Y | yes | YES)
      return 0
      ;;
    *)
      return 1
      ;;
  esac
}

print_launch_instructions() {
  case "$path_action" in
    added)
      step "Current terminal: export PATH=\"$BIN_DIR:\$PATH\" && interpreter"
      step "Future terminals: open a new terminal and run: interpreter"
      step "PATH was added to $path_profile"
      ;;
    updated)
      step "Current terminal: export PATH=\"$BIN_DIR:\$PATH\" && interpreter"
      step "Future terminals: open a new terminal and run: interpreter"
      step "PATH was updated in $path_profile"
      ;;
    configured)
      step "Current terminal: export PATH=\"$BIN_DIR:\$PATH\" && interpreter"
      step "Future terminals: open a new terminal and run: interpreter"
      step "PATH is already configured in $path_profile"
      ;;
    *)
      step "Current terminal: interpreter"
      step "Future terminals: open a new terminal and run: interpreter"
      ;;
  esac
}

maybe_launch_interpreter_now() {
  if prompt_yes_no "Start Open Interpreter now?"; then
    step "Launching Open Interpreter"
    if ( : </dev/tty ) 2>/dev/null; then
      "$BIN_PATH" </dev/tty >/dev/tty 2>/dev/tty
    else
      "$BIN_PATH"
    fi
  fi
}

install_release() {
  release_dir="$1"
  extracted_root="$2"
  stage_release="$RELEASES_DIR/.staging.$(basename "$release_dir").$$"

  mkdir -p "$RELEASES_DIR"
  rm -rf "$stage_release"
  mkdir -p "$stage_release"
  for binary in interpreter interpreter-root-tui interpreter-tui interpreter-app-server codex-exec; do
    cp "$extracted_root/$binary" "$stage_release/$binary"
    chmod 0755 "$stage_release/$binary"
  done

  if [ -e "$release_dir" ] || [ -L "$release_dir" ]; then
    rm -rf "$release_dir"
  fi
  mv "$stage_release" "$release_dir"
}

release_dir_is_complete() {
  release_dir="$1"
  expected_version="$2"
  expected_target="$3"

  [ -d "$release_dir" ] &&
    [ -x "$release_dir/interpreter" ] &&
    [ -x "$release_dir/interpreter-root-tui" ] &&
    [ -x "$release_dir/interpreter-tui" ] &&
    [ -x "$release_dir/interpreter-app-server" ] &&
    [ -x "$release_dir/codex-exec" ] &&
    [ "$(basename "$release_dir")" = "$expected_version-$expected_target" ]
}

update_current_link() {
  release_dir="$1"
  tmp_link="$STANDALONE_ROOT/.current.$$"

  replace_path_with_symlink "$CURRENT_LINK" "$release_dir" "$tmp_link"
}

update_visible_command() {
  mkdir -p "$BIN_DIR"
  tmp_bin="$BIN_DIR/.interpreter.$$"

  rm -f "$tmp_bin"
  {
    printf '%s\n' '#!/bin/sh'
    printf '%s\n' "exec \"$CURRENT_LINK/interpreter\" \"\$@\""
  } >"$tmp_bin"
  chmod 0755 "$tmp_bin"

  if mv -Tf "$tmp_bin" "$BIN_PATH" 2>/dev/null; then
    return
  fi

  if mv -hf "$tmp_bin" "$BIN_PATH" 2>/dev/null; then
    return
  fi

  rm -f "$BIN_PATH"
  mv -f "$tmp_bin" "$BIN_PATH"
}

can_replace_existing_interpreter_command() {
  candidate="$1"

  [ -n "$candidate" ] || return 1
  [ "$candidate" != "$BIN_PATH" ] || return 1
  [ -f "$candidate" ] || return 1
  [ -w "$candidate" ] || return 1

  grep -E '/target/(debug|release)/interpreter|[.]openinterpreter/packages/standalone/current/interpreter' "$candidate" >/dev/null 2>&1
}

existing_interpreter_points_to_current_install() {
  candidate="$1"

  [ -n "$candidate" ] || return 1
  [ -f "$candidate" ] || return 1

  grep -F ".openinterpreter/packages/standalone/current/interpreter" "$candidate" >/dev/null 2>&1
}

handle_existing_interpreter_command() {
  resolved_interpreter="$(command -v interpreter 2>/dev/null || true)"

  if [ -z "$resolved_interpreter" ] || [ "$resolved_interpreter" = "$BIN_PATH" ]; then
    return
  fi

  if existing_interpreter_points_to_current_install "$resolved_interpreter"; then
    return
  fi

  if can_replace_existing_interpreter_command "$resolved_interpreter"; then
    step "An existing interpreter command is already on PATH at $resolved_interpreter"
    if prompt_yes_no "Replace it with this Open Interpreter install?"; then
      tmp_existing="$(dirname "$resolved_interpreter")/.interpreter.$$"
      rm -f "$tmp_existing"
      {
        printf '%s\n' '#!/bin/sh'
        printf '%s\n' "exec \"$CURRENT_LINK/interpreter\" \"\$@\""
      } >"$tmp_existing"
      chmod 0755 "$tmp_existing"
      mv -f "$tmp_existing" "$resolved_interpreter"
      step "Replaced existing interpreter command at $resolved_interpreter"
      return
    fi
  fi

  warn "interpreter currently resolves to $resolved_interpreter before $BIN_PATH. Run: export PATH=\"$BIN_DIR:\$PATH\""
}

verify_visible_command() {
  "$BIN_PATH" --version >/dev/null
}

parse_args "$@"

require_command mktemp
require_command tar

case "$(uname -s)" in
  Darwin)
    os="darwin"
    ;;
  Linux)
    os="linux"
    ;;
  *)
    echo "install.sh supports macOS and Linux. Use install.ps1 on Windows." >&2
    exit 1
    ;;
esac

case "$(uname -m)" in
  x86_64 | amd64)
    arch="x86_64"
    ;;
  arm64 | aarch64)
    arch="aarch64"
    ;;
  *)
    echo "Unsupported architecture: $(uname -m)" >&2
    exit 1
    ;;
esac

if [ "$os" = "darwin" ] && [ "$arch" = "x86_64" ]; then
  if [ "$(sysctl -n sysctl.proc_translated 2>/dev/null || true)" = "1" ]; then
    arch="aarch64"
  fi
fi

if [ "$os" = "darwin" ]; then
  if [ "$arch" = "aarch64" ]; then
    target="aarch64-apple-darwin"
    platform_label="macOS (Apple Silicon)"
  else
    target="x86_64-apple-darwin"
    platform_label="macOS (Intel)"
  fi
else
  if [ "$arch" = "aarch64" ]; then
    target="aarch64-unknown-linux-gnu"
    platform_label="Linux (ARM64)"
  else
    target="x86_64-unknown-linux-gnu"
    platform_label="Linux (x64)"
  fi
fi

resolved_version="$(resolve_version)"
asset="open-interpreter-$target.tar.gz"
download_url="$(release_api_url_for_asset "$asset" "$resolved_version")"
release_name="$resolved_version-$target"
release_dir="$RELEASES_DIR/$release_name"
current_version="$(current_installed_version)"

if [ -n "$current_version" ] && [ "$current_version" != "$resolved_version" ]; then
  step "Updating Open Interpreter from $current_version to $resolved_version"
elif [ -n "$current_version" ]; then
  step "Refreshing Open Interpreter $current_version"
else
  step "Installing Open Interpreter"
fi
step "Detected platform: $platform_label"
step "Resolved version: $resolved_version"

tmp_dir="$(mktemp -d)"
cleanup() {
  release_install_lock
  if [ -n "$tmp_dir" ]; then
    rm -rf "$tmp_dir"
  fi
}
trap cleanup EXIT INT TERM

acquire_install_lock
cleanup_stale_install_artifacts

if ! release_dir_is_complete "$release_dir" "$resolved_version" "$target"; then
  if [ -e "$release_dir" ] || [ -L "$release_dir" ]; then
    warn "Found incomplete existing release at $release_dir; reinstalling."
  fi

  archive_path="$tmp_dir/$asset"
  extract_dir="$tmp_dir/extract"

  step "Downloading Open Interpreter"
  expected_digest="$(release_asset_digest "$asset" "$resolved_version")"
  download_file "$download_url" "$archive_path"
  verify_archive_digest "$archive_path" "$expected_digest"

  mkdir -p "$extract_dir"
  tar -xzf "$archive_path" -C "$extract_dir"

  step "Installing standalone package to $release_dir"
  install_release "$release_dir" "$extract_dir/open-interpreter"
fi

update_current_link "$release_dir"
update_visible_command
handle_existing_interpreter_command
add_to_path
verify_visible_command
release_install_lock

case "$path_action" in
  added | updated | configured)
    print_launch_instructions
    ;;
  *)
    step "$BIN_DIR is already on PATH"
    print_launch_instructions
    ;;
esac

printf 'Open Interpreter %s installed successfully.\n' "$resolved_version"
maybe_launch_interpreter_now
