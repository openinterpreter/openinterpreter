#!/bin/sh

set -eu

RELEASE="${OPEN_INTERPRETER_RELEASE:-${CODEX_RELEASE:-latest}}"
NON_INTERACTIVE="${OPEN_INTERPRETER_NONINTERACTIVE:-${CODEX_NON_INTERACTIVE:-false}}"
GITHUB_REPO="${OPEN_INTERPRETER_GITHUB_REPO:-${CODEX_GITHUB_REPO:-openinterpreter/openinterpreter}}"
PRODUCT_NAME="${CODEX_INSTALL_PRODUCT_NAME:-Open Interpreter}"
PACKAGE_ASSET_STEM="${CODEX_PACKAGE_ASSET_STEM:-open-interpreter-package}"
COMMAND_NAME="${CODEX_COMMAND_NAME:-interpreter}"
ALIAS_COMMAND_NAMES="${CODEX_ALIAS_COMMAND_NAMES:-i}"
RELEASE_TAG_PREFIX="${CODEX_RELEASE_TAG_PREFIX:-rust-v}"

BIN_DIR="${OPEN_INTERPRETER_INSTALL_DIR:-${CODEX_INSTALL_DIR:-$HOME/.local/bin}}"
BIN_PATH="$BIN_DIR/$COMMAND_NAME"
CODE_MODE_HOST_BIN_PATH="$BIN_DIR/codex-code-mode-host"
CODEX_HOME_DIR="${INTERPRETER_HOME:-${CODEX_HOME:-$HOME/.openinterpreter}}"
STANDALONE_ROOT="$CODEX_HOME_DIR/packages/standalone"
RELEASES_DIR="$STANDALONE_ROOT/releases"
CURRENT_LINK="$STANDALONE_ROOT/current"
LOCK_FILE="$STANDALONE_ROOT/install.lock"
LOCK_DIR="$STANDALONE_ROOT/install.lock.d"
LOCK_STALE_AFTER_SECS=600

path_action="already"
path_profile=""
conflict_manager=""
lock_kind=""
tmp_dir=""

step() {
  printf '==> %s\n' "$1"
}

warn() {
  printf 'WARNING: %s\n' "$1" >&2
}

normalize_version() {
  case "$1" in
    "" | latest)
      printf 'latest\n'
      ;;
    rust-v*)
      printf '%s\n' "${1#rust-v}"
      ;;
    v*)
      printf '%s\n' "${1#v}"
      ;;
    *)
      printf '%s\n' "$1"
      ;;
  esac
}

validate_version() {
  version="$1"

  if [ "$version" = "latest" ]; then
    return
  fi

  if ! printf '%s\n' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-(alpha|beta)(\.[0-9]+)?)?$'; then
    echo "Invalid $PRODUCT_NAME release version: $version. Expected latest or x.y.z[-alpha[.N]|-beta[.N]]." >&2
    exit 1
  fi
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
      --help | -h)
        cat <<EOF
Usage: install.sh [--release VERSION]

Environment:
  CODEX_RELEASE          Version to install; overridden by --release.
  CODEX_NON_INTERACTIVE  Set to 1, true, or yes to skip prompts.
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

  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$url" -o "$output"
    return
  fi

  if command -v wget >/dev/null 2>&1; then
    wget -q -O "$output" "$url"
    return
  fi

  echo "curl or wget is required to install $PRODUCT_NAME." >&2
  exit 1
}

download_text() {
  url="$1"

  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$url"
    return
  fi

  if command -v wget >/dev/null 2>&1; then
    wget -q -O - "$url"
    return
  fi

  echo "curl or wget is required to install $PRODUCT_NAME." >&2
  exit 1
}

parse_release_metadata() {
  # Bound awk's record size so compact, single-line JSON stays fast on every
  # supported awk implementation. JSON strings cannot contain literal newlines,
  # so the record boundaries inserted by fold do not change the document.
  LC_ALL=C fold -b -w 4096 | LC_ALL=C awk '
    function finish_string(value) {
      if (object_depth == 1 && key == "tag_name") {
        print "tag_name\t" value
      } else if (object_depth == asset_object_depth) {
        if (key == "name") {
          asset_name = value
        } else if (key == "digest") {
          asset_digest = value
        }
      }

      expecting_value = 0
      key = ""
    }

    {
      for (i = 1; i <= length($0); i++) {
        char = substr($0, i, 1)

        if (in_string) {
          if (escaped) {
            token = token "\\" char
            escaped = 0
          } else if (char == "\\") {
            escaped = 1
          } else if (char == "\"") {
            in_string = 0
            if (string_is_value) {
              finish_string(token)
            } else {
              pending_key = token
            }
          } else {
            token = token char
          }
          continue
        }

        if (char == "\"") {
          in_string = 1
          token = ""
          escaped = 0
          string_is_value = expecting_value
        } else if (char == ":" && pending_key != "") {
          key = pending_key
          pending_key = ""
          expecting_value = 1
        } else if (char == "{") {
          object_depth++
          if (assets_array_depth != 0 &&
              array_depth == assets_array_depth &&
              asset_object_depth == 0) {
            asset_object_depth = object_depth
            asset_name = ""
            asset_digest = ""
          }
          expecting_value = 0
          key = ""
        } else if (char == "}") {
          if (object_depth == asset_object_depth) {
            if (asset_name != "" && asset_digest != "") {
              print "asset\t" asset_name "\t" asset_digest
            }
            asset_object_depth = 0
            asset_name = ""
            asset_digest = ""
          }
          object_depth--
          expecting_value = 0
          key = ""
          pending_key = ""
        } else if (char == "[") {
          array_depth++
          if (expecting_value && key == "assets" && object_depth == 1) {
            assets_array_depth = array_depth
          }
          expecting_value = 0
          key = ""
        } else if (char == "]") {
          if (array_depth == assets_array_depth) {
            assets_array_depth = 0
          }
          array_depth--
          expecting_value = 0
          key = ""
          pending_key = ""
        } else if (char == ",") {
          expecting_value = 0
          key = ""
          pending_key = ""
        }
      }
    }

    END {
      if (in_string || object_depth != 0 || array_depth != 0) {
        exit 1
      }
    }
  '
}

release_url_for_asset() {
  asset="$1"
  resolved_version="$2"

  printf 'https://github.com/%s/releases/download/%s%s/%s\n' "$GITHUB_REPO" "$RELEASE_TAG_PREFIX" "$resolved_version" "$asset"
}

release_metadata_url() {
  resolved_version="$1"

  printf 'https://api.github.com/repos/%s/releases/tags/%s%s\n' "$GITHUB_REPO" "$RELEASE_TAG_PREFIX" "$resolved_version"
}

release_asset_digest_or_empty() {
  asset="$1"

  digest="$(printf '%s\n' "$release_metadata" | awk -F '\t' -v asset="$asset" '
    $1 == "asset" && $2 == asset {
      print $3
      exit
    }
  ')"

  case "$digest" in
    sha256:????????????????????????????????????????????????????????????????)
      printf '%s\n' "${digest#sha256:}"
      ;;
    *)
      return 1
      ;;
  esac
}

release_asset_exists() {
  asset="$1"

  release_asset_digest_or_empty "$asset" >/dev/null 2>&1
}

release_asset_digest() {
  asset="$1"

  digest="$(release_asset_digest_or_empty "$asset" || true)"
  if [ -z "$digest" ]; then
    echo "Could not find SHA-256 digest for release asset $asset." >&2
    exit 1
  fi

  printf '%s\n' "$digest"
}

package_archive_digest() {
  asset="$1"
  manifest_path="$2"

  digest="$(awk -v asset="$asset" '
    $2 == asset && length($1) == 64 && $1 !~ /[^0-9a-fA-F]/ {
      print tolower($1)
      found = 1
      exit
    }
    END {
      if (!found) {
        exit 1
      }
    }
  ' "$manifest_path" 2>/dev/null || true)"

  if [ -z "$digest" ]; then
    echo "Could not find SHA-256 digest for $asset in $checksum_asset." >&2
    exit 1
  fi

  printf '%s\n' "$digest"
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

  echo "sha256sum, shasum, or openssl is required to verify the $PRODUCT_NAME download." >&2
  exit 1
}

verify_archive_digest() {
  archive_path="$1"
  expected_digest="$2"
  actual_digest="$(file_sha256 "$archive_path")"

  if [ "$actual_digest" != "$expected_digest" ]; then
    echo "Downloaded $PRODUCT_NAME archive checksum did not match expected digest." >&2
    echo "expected: $expected_digest" >&2
    echo "actual:   $actual_digest" >&2
    exit 1
  fi
}

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "$1 is required to install $PRODUCT_NAME." >&2
    exit 1
  fi
}

resolve_version() {
  normalized_version="$(normalize_version "$RELEASE")"
  validate_version "$normalized_version"

  if [ "$normalized_version" != "latest" ]; then
    printf '%s\n' "$normalized_version"
    return
  fi

  if [ -z "$RELEASE_TAG_PREFIX" ]; then
    release_json="$(download_text "https://api.github.com/repos/$GITHUB_REPO/releases/latest")"
  else
    release_json="$(download_text "https://api.github.com/repos/$GITHUB_REPO/releases?per_page=100")"
  fi
  tag="$(printf '%s\n' "$release_json" |
    awk -v prefix="$RELEASE_TAG_PREFIX" '
      /"tag_name":[[:space:]]*"[^"]+"/ {
        tag = $0
        sub(/^.*"tag_name":[[:space:]]*"/, "", tag)
        sub(/".*$/, "", tag)
      }

      /"draft":[[:space:]]*(true|false)/ {
        draft = $0
        sub(/^.*"draft":[[:space:]]*/, "", draft)
        sub(/[,[:space:]].*$/, "", draft)
      }

      /"prerelease":[[:space:]]*(true|false)/ {
        prerelease = $0
        sub(/^.*"prerelease":[[:space:]]*/, "", prerelease)
        sub(/[,[:space:]].*$/, "", prerelease)
      }

      tag != "" && draft != "" && prerelease != "" {
        if (draft == "false" && prerelease == "false" && index(tag, prefix) == 1) {
          print tag
          exit
        }
        tag = ""
        draft = ""
        prerelease = ""
      }
    ' | head -n 1)"
  if [ -z "$tag" ]; then
    echo "Failed to resolve the latest $PRODUCT_NAME release version." >&2
    exit 1
  fi
  resolved="$(normalize_version "$tag")"

  if [ -z "$resolved" ] || [ "$resolved" = "latest" ]; then
    echo "Failed to resolve the latest $PRODUCT_NAME release version." >&2
    exit 1
  fi

  validate_version "$resolved"
  printf '%s\n' "$resolved"
}

pick_profile() {
  # Use the same shell-specific split Homebrew documents because there is no
  # universal startup file across macOS/Linux login and interactive shells.
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
      if [ -z "$conflict_manager" ]; then
        return
      fi
      ;;
  esac

  profile="$(pick_profile)"
  path_profile="$profile"
  begin_marker="# >>> $PRODUCT_NAME installer >>>"
  end_marker="# <<< $PRODUCT_NAME installer <<<"
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
    find "$BIN_DIR" -mindepth 1 -maxdepth 1 -name ".$COMMAND_NAME.*" -exec rm -f {} +
    for alias_name in $ALIAS_COMMAND_NAMES; do
      find "$BIN_DIR" -mindepth 1 -maxdepth 1 -name ".$alias_name.*" -exec rm -f {} +
    done
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
  codex_path="$1"

  if [ ! -x "$codex_path" ]; then
    return 1
  fi

  "$codex_path" --version 2>/dev/null | sed -n 's/.* \([0-9][0-9A-Za-z.+-]*\)$/\1/p' | head -n 1
}

current_installed_version() {
  version="$(version_from_binary "$CURRENT_LINK/bin/$COMMAND_NAME" || true)"
  if [ -n "$version" ]; then
    printf '%s\n' "$version"
    return 0
  fi

  version="$(version_from_binary "$CURRENT_LINK/$COMMAND_NAME" || true)"
  if [ -n "$version" ]; then
    printf '%s\n' "$version"
    return 0
  fi

  return 0
}

resolve_existing_codex() {
  command -v codex 2>/dev/null || true
}

classify_existing_codex() {
  existing_path="$1"

  if [ -z "$existing_path" ] || [ "$existing_path" = "$BIN_PATH" ]; then
    return 1
  fi

  case "$existing_path" in
    /opt/homebrew/* | /usr/local/*)
      if [ "$os" = "darwin" ]; then
        printf 'brew\n'
        return 0
      fi
      ;;
  esac

  if [ -f "$existing_path" ] && grep -F "#!/usr/bin/env node" "$existing_path" >/dev/null 2>&1; then
    case "$existing_path" in
      *".bun"*)
        printf 'bun\n'
        ;;
      *)
        printf 'npm\n'
        ;;
    esac
    return 0
  fi

  return 1
}

prompt_yes_no() {
  prompt="$1"

  case "$NON_INTERACTIVE" in
    1 | [Tt][Rr][Uu][Ee] | [Yy][Ee][Ss])
      return 1
      ;;
  esac

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
  if [ "$COMMAND_NAME" = "interpreter" ]; then
    start_command="i or interpreter"
  else
    start_command="$COMMAND_NAME"
  fi

  case "$path_action" in
    added)
      step "Current terminal: export PATH=\"$BIN_DIR:\$PATH\" then run: $start_command"
      step "Future terminals: open a new terminal and run: $start_command"
      step "PATH was added to $path_profile"
      ;;
    updated)
      step "Current terminal: export PATH=\"$BIN_DIR:\$PATH\" then run: $start_command"
      step "Future terminals: open a new terminal and run: $start_command"
      step "PATH was updated in $path_profile"
      ;;
    configured)
      step "Current terminal: export PATH=\"$BIN_DIR:\$PATH\" then run: $start_command"
      step "Future terminals: open a new terminal and run: $start_command"
      step "PATH is already configured in $path_profile"
      ;;
    *)
      step "Current terminal: $start_command"
      step "Future terminals: open a new terminal and run: $start_command"
      ;;
  esac
}

maybe_launch_command_now() {
  if prompt_yes_no "Start $PRODUCT_NAME now?"; then
    step "Launching $PRODUCT_NAME"
    "$BIN_PATH"
  fi
}

detect_conflicting_install() {
  if [ "$COMMAND_NAME" != "codex" ]; then
    return
  fi

  existing_path="$(resolve_existing_codex)"
  manager="$(classify_existing_codex "$existing_path" || true)"

  if [ -z "$manager" ]; then
    return
  fi

  conflict_manager="$manager"
  step "Detected existing $manager-managed Codex at $existing_path"
  warn "Multiple managed Codex installs can be ambiguous because PATH order decides which one runs."
}

handle_conflicting_install() {
  if [ -z "$conflict_manager" ]; then
    return
  fi

  case "$conflict_manager" in
    brew)
      uninstall_cmd="brew uninstall --cask codex"
      ;;
    bun)
      uninstall_cmd="bun remove -g @openai/codex"
      ;;
    *)
      uninstall_cmd="npm uninstall -g @openai/codex"
      ;;
  esac

  if prompt_yes_no "Uninstall the existing $conflict_manager-managed Codex now?"; then
    step "Running: $uninstall_cmd"
    if ! sh -c "$uninstall_cmd"; then
      warn "Failed to uninstall the existing $conflict_manager-managed Codex. Continuing with the standalone install."
    fi
  else
    warn "Leaving the existing $conflict_manager-managed Codex installed. PATH order will determine which codex runs."
  fi
}

install_package_release() {
  release_dir="$1"
  archive_path="$2"
  stage_release="$RELEASES_DIR/.staging.$(basename "$release_dir").$$"

  mkdir -p "$RELEASES_DIR"
  rm -rf "$stage_release"
  mkdir -p "$stage_release"
  tar -xzf "$archive_path" -C "$stage_release"
  find "$stage_release/bin" -mindepth 1 -maxdepth 1 -type f -exec chmod 0755 {} +
  chmod 0755 "$stage_release/codex-path/rg"
  if [ -f "$stage_release/codex-resources/bwrap" ]; then
    chmod 0755 "$stage_release/codex-resources/bwrap"
  fi
  if [ -e "$release_dir" ] || [ -L "$release_dir" ]; then
    rm -rf "$release_dir"
  fi
  mv "$stage_release" "$release_dir"
}

install_legacy_platform_npm_release() {
  release_dir="$1"
  archive_path="$2"
  target="$3"
  stage_release="$RELEASES_DIR/.staging.$(basename "$release_dir").$$"
  extract_dir="$tmp_dir/extract"
  vendor_root="$extract_dir/package/vendor/$target"

  mkdir -p "$RELEASES_DIR"
  rm -rf "$stage_release" "$extract_dir"
  mkdir -p "$stage_release/codex-resources" "$extract_dir"
  tar -xzf "$archive_path" -C "$extract_dir"

  cp "$vendor_root/codex/codex" "$stage_release/codex"
  cp "$vendor_root/path/rg" "$stage_release/codex-resources/rg"
  chmod 0755 "$stage_release/codex" "$stage_release/codex-resources/rg"
  if [ -f "$vendor_root/codex-resources/bwrap" ]; then
    cp "$vendor_root/codex-resources/bwrap" "$stage_release/codex-resources/bwrap"
    chmod 0755 "$stage_release/codex-resources/bwrap"
  fi

  if [ -e "$release_dir" ] || [ -L "$release_dir" ]; then
    rm -rf "$release_dir"
  fi
  mv "$stage_release" "$release_dir"
}

release_dir_is_complete() {
  release_dir="$1"
  expected_version="$2"
  expected_target="$3"
  layout="$4"

  [ -d "$release_dir" ] &&
    [ "$(basename "$release_dir")" = "$expected_version-$expected_target" ] ||
    return 1

  case "$layout" in
    package)
      [ -f "$release_dir/codex-package.json" ] &&
        [ -x "$release_dir/$(package_entrypoint_relative_path "$release_dir")" ] &&
        [ -x "$release_dir/bin/codex-code-mode-host" ] &&
        [ -x "$release_dir/codex-path/rg" ] ||
        return 1
      ;;
    legacy-platform-npm)
      [ -x "$release_dir/codex" ] &&
        [ -x "$release_dir/codex-resources/rg" ] ||
        return 1
      ;;
    *)
      return 1
      ;;
  esac

  case "$layout:$expected_target" in
    package:*linux* | legacy-platform-npm:*linux*) [ -x "$release_dir/codex-resources/bwrap" ] ;;
    *) true ;;
  esac
}

update_current_link() {
  release_dir="$1"
  tmp_link="$STANDALONE_ROOT/.current.$$"

  replace_path_with_symlink "$CURRENT_LINK" "$release_dir" "$tmp_link"
}

package_entrypoint_relative_path() {
  release_dir="$1"
  entrypoint="$(awk -F '"' '/"entrypoint":[[:space:]]*"/ { print $4; exit }' "$release_dir/codex-package.json" 2>/dev/null || true)"
  if [ -n "$entrypoint" ] && [ -x "$release_dir/$entrypoint" ]; then
    printf '%s\n' "$entrypoint"
    return
  fi

  if [ -x "$release_dir/bin/$COMMAND_NAME" ]; then
    printf 'bin/%s\n' "$COMMAND_NAME"
  else
    printf '%s\n' "$COMMAND_NAME"
  fi
}

resolve_dir_realpath() {
  dir="$1"

  ( cd -P "$dir" 2>/dev/null && pwd -P ) || true
}

reject_self_referential_install_dir() {
  release_dir="$1"
  command_relative_path="$2"
  managed_dir="$release_dir/$(dirname "$command_relative_path")"

  managed_dir_real="$(resolve_dir_realpath "$managed_dir")"
  bin_dir_real="$(resolve_dir_realpath "$BIN_DIR")"

  if [ -n "$managed_dir_real" ] && [ "$managed_dir_real" = "$bin_dir_real" ]; then
    echo "CODEX_INSTALL_DIR ($BIN_DIR) resolves to this release's managed executable directory ($managed_dir_real)." >&2
    echo "Installing here would overwrite the managed executable with a self-referential symlink. Choose a different CODEX_INSTALL_DIR." >&2
    exit 1
  fi
}

update_visible_command() {
  release_dir="$1"
  mkdir -p "$BIN_DIR"
  tmp_link="$BIN_DIR/.$COMMAND_NAME.$$"
  command_relative_path="$(package_entrypoint_relative_path "$release_dir")"

  reject_self_referential_install_dir "$release_dir" "$command_relative_path"

  replace_path_with_symlink "$BIN_PATH" "$CURRENT_LINK/$command_relative_path" "$tmp_link"

  for alias_name in $ALIAS_COMMAND_NAMES; do
    tmp_alias_link="$BIN_DIR/.$alias_name.$$"
    replace_path_with_symlink "$BIN_DIR/$alias_name" "$CURRENT_LINK/$command_relative_path" "$tmp_alias_link"
  done

  if [ "$os" = "darwin" ] && [ -x "$release_dir/bin/codex-code-mode-host" ]; then
    tmp_host_link="$BIN_DIR/.codex-code-mode-host.$$"
    replace_path_with_symlink \
      "$CODE_MODE_HOST_BIN_PATH" \
      "$CURRENT_LINK/bin/codex-code-mode-host" \
      "$tmp_host_link"
  elif [ "$(readlink "$CODE_MODE_HOST_BIN_PATH" 2>/dev/null || true)" = \
    "$CURRENT_LINK/bin/codex-code-mode-host" ]; then
    rm -f "$CODE_MODE_HOST_BIN_PATH"
  fi
}

verify_visible_command() {
  "$BIN_PATH" --version >/dev/null
  for alias_name in $ALIAS_COMMAND_NAMES; do
    "$BIN_DIR/$alias_name" --version >/dev/null
  done
  if [ "$os" = "darwin" ] && [ "$install_layout" = "package" ]; then
    [ -x "$CODE_MODE_HOST_BIN_PATH" ]
  fi
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
    npm_tag="darwin-arm64"
    vendor_target="aarch64-apple-darwin"
    platform_label="macOS (Apple Silicon)"
  else
    npm_tag="darwin-x64"
    vendor_target="x86_64-apple-darwin"
    platform_label="macOS (Intel)"
  fi
else
  if [ "$arch" = "aarch64" ]; then
    npm_tag="linux-arm64"
    vendor_target="aarch64-unknown-linux-musl"
    platform_label="Linux (ARM64)"
  else
    npm_tag="linux-x64"
    vendor_target="x86_64-unknown-linux-musl"
    platform_label="Linux (x64)"
  fi
fi

resolved_version="$(resolve_version)"
if ! release_json="$(download_text "$(release_metadata_url "$resolved_version")")"; then
  echo "Could not fetch GitHub release metadata for $PRODUCT_NAME $resolved_version. GitHub API may be unavailable or rate limited." >&2
  exit 1
fi
if ! release_metadata="$(printf '%s\n' "$release_json" | parse_release_metadata)"; then
  echo "Could not parse GitHub release metadata for $PRODUCT_NAME $resolved_version." >&2
  exit 1
fi
package_asset="$PACKAGE_ASSET_STEM-$vendor_target.tar.gz"
checksum_asset="codex-package_SHA256SUMS"
if release_asset_exists "$package_asset" &&
  release_asset_exists "$checksum_asset"; then
  install_layout="package"
  asset="$package_asset"
elif [ "$PACKAGE_ASSET_STEM" = "codex-package" ] &&
  release_asset_exists "codex-npm-$npm_tag-$resolved_version.tgz"; then
  install_layout="legacy-platform-npm"
  asset="codex-npm-$npm_tag-$resolved_version.tgz"
else
  echo "Could not find $PRODUCT_NAME release assets for $resolved_version." >&2
  exit 1
fi
download_url="$(release_url_for_asset "$asset" "$resolved_version")"
checksum_url="$(release_url_for_asset "$checksum_asset" "$resolved_version")"
release_name="$resolved_version-$vendor_target"
release_dir="$RELEASES_DIR/$release_name"
current_version="$(current_installed_version)"

if [ -n "$current_version" ] && [ "$current_version" != "$resolved_version" ]; then
  step "Updating $PRODUCT_NAME from $current_version to $resolved_version"
elif [ -n "$current_version" ]; then
  step "Updating $PRODUCT_NAME"
else
  step "Installing $PRODUCT_NAME"
fi
step "Detected platform: $platform_label"
step "Resolved version: $resolved_version"

detect_conflicting_install

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

if ! release_dir_is_complete "$release_dir" "$resolved_version" "$vendor_target" "$install_layout"; then
  if [ -e "$release_dir" ] || [ -L "$release_dir" ]; then
    warn "Found incomplete existing release at $release_dir; reinstalling."
  fi

  archive_path="$tmp_dir/$asset"
  checksum_path="$tmp_dir/$checksum_asset"

  step "Downloading $PRODUCT_NAME"
  if [ "$install_layout" = "package" ]; then
    checksum_digest="$(release_asset_digest "$checksum_asset")"
    download_file "$checksum_url" "$checksum_path"
    verify_archive_digest "$checksum_path" "$checksum_digest"
    expected_digest="$(package_archive_digest "$asset" "$checksum_path")"
  else
    expected_digest="$(release_asset_digest "$asset")"
  fi
  download_file "$download_url" "$archive_path"
  verify_archive_digest "$archive_path" "$expected_digest"

  step "Installing standalone package to $release_dir"
  if [ "$install_layout" = "package" ]; then
    install_package_release "$release_dir" "$archive_path"
  else
    install_legacy_platform_npm_release "$release_dir" "$archive_path" "$vendor_target"
  fi
fi
update_current_link "$release_dir"
update_visible_command "$release_dir"
add_to_path
verify_visible_command
release_install_lock
handle_conflicting_install

case "$path_action" in
  added)
    print_launch_instructions
    ;;
  updated)
    print_launch_instructions
    ;;
  configured)
    print_launch_instructions
    ;;
  *)
    step "$BIN_DIR is already on PATH"
    print_launch_instructions
    ;;
esac

printf '%s %s installed successfully.\n' "$PRODUCT_NAME" "$resolved_version"
maybe_launch_command_now
