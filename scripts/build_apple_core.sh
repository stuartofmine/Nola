#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Builds the Nola v2 Rust Core static library (`libnola_app.a`) for Apple platforms
# (macOS, iOS, iOS Simulator) and installs it where NolaCore.xcodeproj expects `libnola_core.a`.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd -P)"
. "$ROOT/scripts/lib/rust_build_cache.sh"

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
  echo "error: cargo not found; install rustup or set CARGO" >&2
  exit 127
fi
nola_rust_build_cache_init "$CARGO_BIN"

export CARGO_TARGET_DIR="${NOLA_CORE_CARGO_TARGET_DIR:-${CARGO_TARGET_DIR:-$ROOT/target}}"
PROFILE="${NOLA_CARGO_PROFILE:-}"
if [[ -z "$PROFILE" ]]; then
  case "${CONFIGURATION:-Debug}" in
    Release) PROFILE="release" ;;
    *) PROFILE="dev" ;;
  esac
fi
OUTPUT_PROFILE="$PROFILE"
[[ "$OUTPUT_PROFILE" == "dev" ]] && OUTPUT_PROFILE="debug"

resolve_target_triple() {
  local sdk="${PLATFORM_NAME:-macosx}"
  local arch="${1:-$(uname -m)}"
  case "$sdk:$arch" in
    iphoneos:arm64) echo "aarch64-apple-ios" ;;
    iphonesimulator:arm64) echo "aarch64-apple-ios-sim" ;;
    iphonesimulator:x86_64) echo "x86_64-apple-ios" ;;
    macosx:arm64) echo "aarch64-apple-darwin" ;;
    macosx:x86_64) echo "x86_64-apple-darwin" ;;
    *) echo "error: unsupported Apple target $sdk:$arch" >&2; exit 1 ;;
  esac
}

OUT_DIR="${DERIVED_FILE_DIR:-$CARGO_TARGET_DIR/apple}/nola_core"
mkdir -p "$OUT_DIR"
OUT_LIB="$OUT_DIR/libnola_core.a"

ARCH_LIST="${ARCHS:-$(uname -m)}"
BUILT_LIBS=()
for arch in $ARCH_LIST; do
  TRIPLE="$(resolve_target_triple "$arch")"
  if [[ "$PROFILE" == "release" ]]; then
    export CARGO_INCREMENTAL=0
  fi
  nola_cargo build \
    --locked \
    --manifest-path "$ROOT/Cargo.toml" \
    --profile "$PROFILE" \
    --target "$TRIPLE" \
    -p nola-app \
    --lib
  LIB_PATH="$CARGO_TARGET_DIR/$TRIPLE/$OUTPUT_PROFILE/libnola_app.a"
  [[ -f "$LIB_PATH" ]] || { echo "error: missing $LIB_PATH" >&2; exit 2; }
  BUILT_LIBS+=("$LIB_PATH")
done

if [[ "${#BUILT_LIBS[@]}" -eq 1 ]]; then
  cp -f "${BUILT_LIBS[0]}" "$OUT_LIB"
else
  lipo -create "${BUILT_LIBS[@]}" -output "$OUT_LIB"
fi
printf '%s\n' "$PROFILE" > "$OUT_DIR/cargo-profile"

if [[ -n "${NOLA_DEPENDENCY_FILE:-}" ]]; then
  mkdir -p "$(dirname "$NOLA_DEPENDENCY_FILE")"
  {
    printf '%s:' "$(printf '%s' "$OUT_LIB" | sed 's/[\\ #]/\\&/g')"
    {
      find "$ROOT/crates" -type f \( -name '*.rs' -o -name '*.h' -o -name 'Cargo.toml' \) -print
      printf '%s\n' "$ROOT/Cargo.toml" "$ROOT/Cargo.lock" "$ROOT/rust-toolchain.toml" "$ROOT/scripts/build_apple_core.sh"
    } | LC_ALL=C sort -u | sed 's/[\\ #]/\\&/g' | awk '{ printf " %s", $0 }'
    printf '\n'
  } > "$NOLA_DEPENDENCY_FILE"
fi
