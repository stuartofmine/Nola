#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Builds Nola v2 Rust Core static library (`libnola_app.a`) for Android ABIs
# and installs each archive at `<output_dir>/<abi>/libnola_core.a` for CMake/JNI.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd -P)"
OUTPUT_DIR="${1:?Usage: build_android_core.sh <output_dir>}"
ABIS_RAW="${NOLA_ANDROID_ABIS:-arm64-v8a,x86_64}"
PROFILE="${NOLA_ANDROID_CARGO_PROFILE:-release}"
OUTPUT_PROFILE="$PROFILE"
[[ "$OUTPUT_PROFILE" == "dev" ]] && OUTPUT_PROFILE="debug"

. "$ROOT/scripts/lib/rust_build_cache.sh"
nola_rust_build_cache_init "${CARGO:-cargo}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"

IFS=',; ' read -r -a ABIS <<< "$ABIS_RAW"
for abi in "${ABIS[@]}"; do
  [[ -z "$abi" ]] && continue
  case "$abi" in
    arm64-v8a)   TRIPLE="aarch64-linux-android" ;;
    x86_64)      TRIPLE="x86_64-linux-android" ;;
    armeabi-v7a) TRIPLE="armv7-linux-androideabi" ;;
    x86)         TRIPLE="i686-linux-android" ;;
    *) echo "error: unsupported Android ABI '$abi'" >&2; exit 1 ;;
  esac

  if command -v cargo-ndk >/dev/null 2>&1; then
    (cd "$ROOT" && cargo ndk -t "$abi" -p 26 -- build --locked --profile "$PROFILE" -p nola-app --lib)
  else
    nola_cargo build \
      --locked \
      --manifest-path "$ROOT/Cargo.toml" \
      --profile "$PROFILE" \
      --target "$TRIPLE" \
      -p nola-app \
      --lib
  fi

  SRC_LIB="$CARGO_TARGET_DIR/$TRIPLE/$OUTPUT_PROFILE/libnola_app.a"
  [[ -f "$SRC_LIB" ]] || { echo "error: expected Rust staticlib at $SRC_LIB" >&2; exit 2; }
  mkdir -p "$OUTPUT_DIR/$abi"
  cp -f "$SRC_LIB" "$OUTPUT_DIR/$abi/libnola_core.a"
done
