#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Builds the two native executables embedded by NolaMac without coupling the
# application target to the complete CLI product.
set -euo pipefail

ROOT="${NOLA_REPOSITORY_ROOT:-${SRCROOT:?SRCROOT is required}}"
NOLA_RUST_BUILD_CACHE_HELPER="$(cd "$(dirname "$0")" && pwd -P)/lib/rust_build_cache.sh"
. "$NOLA_RUST_BUILD_CACHE_HELPER"
NOLA_MAC_INSTALL_HELPER="$(dirname "$NOLA_RUST_BUILD_CACHE_HELPER")/install_mac_executable.sh"
. "$NOLA_MAC_INSTALL_HELPER"
CARGO_BIN="${CARGO:-}"
if [[ -n "$CARGO_BIN" ]]; then
  CARGO_BIN="$(command -v "$CARGO_BIN" 2>/dev/null || printf '%s' "$CARGO_BIN")"
else
  CARGO_BIN="$(command -v cargo 2>/dev/null || true)"
fi
if [[ -z "$CARGO_BIN" && -n "${SHELL:-}" && -x "${SHELL}" ]]; then
  CARGO_BIN="$("${SHELL}" -lc 'command -v cargo' 2>/dev/null || true)"
fi
if [[ -z "$CARGO_BIN" || ! -x "$CARGO_BIN" ]]; then
  echo "error: cargo was not found in the current or login-shell environment. Set CARGO to override it." >&2
  exit 127
fi
nola_rust_build_cache_init "$CARGO_BIN"

export CARGO_TARGET_DIR="${NOLA_AGENT_CARGO_TARGET_DIR:-${CARGO_TARGET_DIR:-${ROOT}/target}}"
BUILD_PROFILE="${NOLA_BUILD_PROFILE:-}"
if [[ -z "$BUILD_PROFILE" ]]; then
  case "${CONFIGURATION:-Debug}" in
    Release) BUILD_PROFILE="release" ;;
    *) BUILD_PROFILE="dev" ;;
  esac
fi
OUTPUT_PROFILE="$BUILD_PROFILE"
[[ "$OUTPUT_PROFILE" == "dev" ]] && OUTPUT_PROFILE="debug"
HOST_ARTIFACT="${CARGO_TARGET_DIR}/${OUTPUT_PROFILE}/nola-agent-host"
SERVICE_ARTIFACT="${CARGO_TARGET_DIR}/${OUTPUT_PROFILE}/nola-service"

mkdir -p "$CARGO_TARGET_DIR"

escape_dependency_path() {
  printf '%s' "$1" | sed 's/[\\ #]/\\&/g'
}

write_dependency_file() {
  local dependency_file="${NOLA_DEPENDENCY_FILE:-}"
  [[ -z "$dependency_file" ]] && return
  mkdir -p "$(dirname "$dependency_file")"
  {
    printf '%s:' "$(escape_dependency_path "$HOST_DEST")"
    {
      find \
        "$ROOT/apps/cli/src" \
        "$ROOT/apps/cli/vendor" \
        "$ROOT/core" \
        "$ROOT/third_party/a2ui/spec" \
        -type d \( -name target -o -name 'target_*' -o -name tests \) -prune -o \
        -type f \( -name '*.rs' -o -name '*.json' -o -name 'Cargo.toml' -o -name 'build.rs' -o -name '*.sql' -o -name '*.py' \) \
        -print
      find "$ROOT/apps/macos/Resources/NolaBuiltInSkills.bundle" \
        -type f ! -name '*.pyc' -print
      printf '%s\n' \
        "$ROOT/Cargo.toml" \
        "$ROOT/Cargo.lock" \
        "$ROOT/rust-toolchain.toml" \
        "$ROOT/apps/cli/build.rs" \
        "$ROOT/scripts/build_mac_runtime.sh" \
        "$NOLA_RUST_BUILD_CACHE_HELPER" \
        "$NOLA_MAC_INSTALL_HELPER" \
        "$ROOT/apps/macos/Resources/com.gaojc.Nola.background-scheduler.plist"
    } | LC_ALL=C sort -u | sed 's/[\\ #]/\\&/g' | awk '{ printf " %s", $0 }'
    printf '\n'
  } > "$dependency_file"
}

# Xcode decides when this phase runs from the generated dependency file. Once
# invoked, Cargo remains the source of truth for the transitive Rust graph.
# Do not use executable mtimes to bypass Cargo: they cannot detect toolchain,
# feature or profile changes. Cargo cheaply reuses valid fingerprints itself.
{
  if [[ "$BUILD_PROFILE" == "release" ]]; then
    export CARGO_INCREMENTAL=0
  fi
  CARGO_ARGS=(
    build
    --locked
    --profile "$BUILD_PROFILE"
    --manifest-path "$ROOT/Cargo.toml"
    --no-default-features
    --features nola_cli/native-host
    -p nola_cli
    --bin nola-agent-host
    -p nola_service
    --bin nola-service
  )
  # Job control gives only this background build a private process group. A
  # new shell verifies it is the group leader before executing Cargo/mbx, so
  # failed group creation cannot start an uncontained build. Positional args
  # retain executable/argument boundaries; no user text is shell-evaluated.
  set -m
  "$BASH" -c '
    . "$1"
    NOLA_RUST_CARGO_COMMAND=$2
    NOLA_RUST_CACHE_BACKEND=$3
    NOLA_RUST_CACHE_MBX=$4
    shift 4
    nola_cargo_exec_owned_group "$@"
  ' nola-rust-build "$NOLA_RUST_BUILD_CACHE_HELPER" "$NOLA_RUST_CARGO_COMMAND" \
    "$NOLA_RUST_CACHE_BACKEND" "$NOLA_RUST_CACHE_MBX" "${CARGO_ARGS[@]}" &
  CARGO_PID=$!
  set +m
  cleanup_cargo() {
    if kill -0 "$CARGO_PID" 2>/dev/null; then
      # Cargo is a child of mbx when caching is selected. Signal only the
      # verified private group, including its compiler descendants, not names
      # or an inferred ancestry shared with another build or the Xcode process.
      local owned_group
      owned_group="$(nola_rust_process_group "$CARGO_PID" 2>/dev/null)" || owned_group=""
      if [[ "$owned_group" == "$CARGO_PID" ]]; then
        kill -TERM -- "-$CARGO_PID" 2>/dev/null || true
      else
        echo "error: private Rust build group identity unavailable; stopping only its owned leader." >&2
        kill -TERM "$CARGO_PID" 2>/dev/null || true
      fi
      wait "$CARGO_PID" 2>/dev/null || true
    fi
  }
  trap cleanup_cargo EXIT INT TERM
  wait "$CARGO_PID"
  trap - EXIT INT TERM
}

RUNTIME_OUTPUT_DIR="${NOLA_RUNTIME_OUTPUT_DIR:-${TARGET_BUILD_DIR}/${CONTENTS_FOLDER_PATH}/MacOS}"
HOST_DEST="${RUNTIME_OUTPUT_DIR}/nola-agent-host"
SERVICE_DEST="${RUNTIME_OUTPUT_DIR}/nola-service"
mkdir -p "$(dirname "$HOST_DEST")"
INSTALL_SIGN=NO
if [[ -z "${NOLA_RUNTIME_OUTPUT_DIR:-}" && "${CODE_SIGNING_ALLOWED:-NO}" == "YES" ]]; then
  INSTALL_SIGN=YES
fi
nola_install_mac_executable "$HOST_ARTIFACT" "$HOST_DEST" "$INSTALL_SIGN"
nola_install_mac_executable "$SERVICE_ARTIFACT" "$SERVICE_DEST" "$INSTALL_SIGN"

AGENT_PLIST_SOURCE="$ROOT/apps/macos/Resources/com.gaojc.Nola.background-scheduler.plist"
if [[ -n "${NOLA_RUNTIME_OUTPUT_DIR:-}" ]]; then
  AGENT_PLIST_DEST="${RUNTIME_OUTPUT_DIR}/com.gaojc.Nola.background-scheduler.plist"
else
  AGENT_PLIST_DEST="${TARGET_BUILD_DIR}/${CONTENTS_FOLDER_PATH}/Library/LaunchAgents/com.gaojc.Nola.background-scheduler.plist"
fi
mkdir -p "$(dirname "$AGENT_PLIST_DEST")"
cp "$AGENT_PLIST_SOURCE" "$AGENT_PLIST_DEST"
chmod 644 "$AGENT_PLIST_DEST"
write_dependency_file
