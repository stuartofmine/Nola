#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Builds nola_core static archives for Android NDK ABIs.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd -P)"
. "$ROOT/../scripts/lib/rust_build_cache.sh"
nola_rust_build_cache_init "${CARGO:-cargo}"
BINDINGS_MANIFEST="$ROOT/nola-mobile-bindings/Cargo.toml"
OUTPUT_DIR="${1:-$ROOT/../apps/android/app/src/main/cpp/rust}"
MIN_SDK="${NOLA_ANDROID_MIN_SDK:-26}"

if ! nola_cargo ndk --version >/dev/null 2>&1; then
  echo "cargo-ndk is required. Install it explicitly with: cargo install cargo-ndk" >&2
  exit 1
fi
NDK_PATH="${ANDROID_NDK_HOME:-${ANDROID_NDK_ROOT:-}}"
if [[ -z "$NDK_PATH" ]]; then
  SDK_PATH="${ANDROID_SDK_ROOT:-${ANDROID_HOME:-}}"
  if [[ -z "$SDK_PATH" && -f "$ROOT/../apps/android/local.properties" ]]; then
    SDK_PATH=$(sed -n 's/^sdk\.dir=//p' "$ROOT/../apps/android/local.properties" | tail -1 | sed 's/\\:/:/g; s/\\\\/\\/g')
  fi
  if [[ -n "$SDK_PATH" && -d "$SDK_PATH/ndk" ]]; then
    NDK_PATH=$(find "$SDK_PATH/ndk" -mindepth 1 -maxdepth 1 -type d | sort | tail -1)
  fi
fi
if [[ -z "$NDK_PATH" || ! -d "$NDK_PATH" ]]; then
  echo "No Android NDK was found. Install one with SDK Manager or set ANDROID_NDK_HOME." >&2
  exit 1
fi
export ANDROID_NDK_HOME="$NDK_PATH"
export ANDROID_NDK_ROOT="$NDK_PATH"

ABIS=(
  "arm64-v8a:aarch64-linux-android"
  "x86_64:x86_64-linux-android"
)

if [[ -n "${NOLA_ANDROID_ABIS:-}" ]]; then
  requested_abis="${NOLA_ANDROID_ABIS//,/ }"
  selected_abis=()
  for requested_abi in $requested_abis; do
    matched=""
    for pair in "${ABIS[@]}"; do
      if [[ "${pair%%:*}" == "$requested_abi" ]]; then
        matched="$pair"
        break
      fi
    done
    if [[ -z "$matched" ]]; then
      echo "Unsupported Android ABI in NOLA_ANDROID_ABIS: $requested_abi" >&2
      exit 2
    fi
    selected_abis+=("$matched")
  done
  if [[ "${#selected_abis[@]}" -eq 0 ]]; then
    echo "NOLA_ANDROID_ABIS did not contain an Android ABI." >&2
    exit 2
  fi
  ABIS=("${selected_abis[@]}")
fi

for pair in "${ABIS[@]}"; do
  IFS=: read -r abi target <<<"$pair"
  echo "Building nola_core for $abi ($target)"
  (
    cd "$ROOT"
    CARGO_TARGET_DIR="$ROOT/target" nola_cargo ndk -t "$abi" -P "$MIN_SDK" build --manifest-path "$BINDINGS_MANIFEST" --release
  )
  archive="$ROOT/target/$target/release/libnola_mobile_bindings.a"
  if [[ ! -f "$archive" ]]; then
    echo "Expected static archive was not produced: $archive" >&2
    exit 1
  fi
  mkdir -p "$OUTPUT_DIR/$abi"
  # A Cargo freshness check must not force an unchanged JNI library to relink.
  if ! cmp -s "$archive" "$OUTPUT_DIR/$abi/libnola_core.a"; then
    cp "$archive" "$OUTPUT_DIR/$abi/libnola_core.a"
  fi
done

echo "Android Rust static archives copied to $OUTPUT_DIR"
