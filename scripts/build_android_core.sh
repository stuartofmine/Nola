#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Builds Nola v2 Rust Core static library (`libnola_app.a`) for Android ABIs
# and installs each archive at `<output_dir>/<abi>/libnola_core.a` for CMake/JNI.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd -P)"
OUTPUT_DIR="${1:?Usage: build_android_core.sh <output_dir>}"
ABIS_RAW="${NOLA_ANDROID_ABIS:-arm64-v8a,x86_64}"
PROFILE="${NOLA_ANDROID_CARGO_PROFILE:-release}"
MIN_API="${NOLA_ANDROID_MIN_API:-26}"
OUTPUT_PROFILE="$PROFILE"
[[ "$OUTPUT_PROFILE" == "dev" ]] && OUTPUT_PROFILE="debug"

export PATH="${HOME}/.cargo/bin:${PATH}"
. "$ROOT/scripts/lib/rust_build_cache.sh"
nola_rust_build_cache_init "${CARGO:-cargo}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"

resolve_ndk_dir() {
  for cand in "${ANDROID_NDK_HOME:-}" "${ANDROID_NDK_ROOT:-}" "${NDK_HOME:-}"; do
    if [[ -n "$cand" && -d "$cand/toolchains/llvm/prebuilt" ]]; then
      printf '%s' "$cand"
      return 0
    fi
  done
  local sdk="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Android/Sdk}}"
  if [[ -d "$sdk/ndk" ]]; then
    local latest
    latest="$(ls -1d "$sdk/ndk"/* 2>/dev/null | sort -V | tail -n 1 || true)"
    if [[ -n "$latest" && -d "$latest/toolchains/llvm/prebuilt" ]]; then
      printf '%s' "$latest"
      return 0
    fi
  fi
  return 1
}

NDK_DIR="$(resolve_ndk_dir || true)"
HOST_TAG="$(uname -s | tr '[:upper:]' '[:lower:]')-$(uname -m)"
[[ "$HOST_TAG" == "darwin-arm64" ]] && HOST_TAG="darwin-x86_64"
TOOLCHAIN_BIN="${NDK_DIR}/toolchains/llvm/prebuilt/${HOST_TAG}/bin"

IFS=',; ' read -r -a ABIS <<< "$ABIS_RAW"
for abi in "${ABIS[@]}"; do
  [[ -z "$abi" ]] && continue
  case "$abi" in
    arm64-v8a)
      TRIPLE="aarch64-linux-android"
      CLANG_PREFIX="aarch64-linux-android${MIN_API}"
      ENV_KEY="AARCH64_LINUX_ANDROID"
      CC_KEY="aarch64_linux_android"
      ;;
    x86_64)
      TRIPLE="x86_64-linux-android"
      CLANG_PREFIX="x86_64-linux-android${MIN_API}"
      ENV_KEY="X86_64_LINUX_ANDROID"
      CC_KEY="x86_64_linux_android"
      ;;
    armeabi-v7a)
      TRIPLE="armv7-linux-androideabi"
      CLANG_PREFIX="armv7a-linux-androideabi${MIN_API}"
      ENV_KEY="ARMV7_LINUX_ANDROIDEABI"
      CC_KEY="armv7_linux_androideabi"
      ;;
    x86)
      TRIPLE="i686-linux-android"
      CLANG_PREFIX="i686-linux-android${MIN_API}"
      ENV_KEY="I686_LINUX_ANDROID"
      CC_KEY="i686_linux_android"
      ;;
    *)
      echo "error: unsupported Android ABI '$abi'" >&2
      exit 1
      ;;
  esac

  if command -v cargo-ndk >/dev/null 2>&1; then
    (cd "$ROOT" && nola_cargo ndk -t "$abi" -p "$MIN_API" -- build --locked --profile "$PROFILE" -p nola-app --lib)
  else
    [[ -x "$TOOLCHAIN_BIN/${CLANG_PREFIX}-clang" ]] || {
      echo "error: Android NDK clang not found at $TOOLCHAIN_BIN/${CLANG_PREFIX}-clang (set ANDROID_NDK_HOME or ANDROID_HOME)" >&2
      exit 1
    }
    CC_VAR="CC_${CC_KEY}"
    CXX_VAR="CXX_${CC_KEY}"
    AR_VAR="AR_${CC_KEY}"
    RANLIB_VAR="RANLIB_${CC_KEY}"
    LINKER_VAR="CARGO_TARGET_${ENV_KEY}_LINKER"
    TARGET_AR_VAR="CARGO_TARGET_${ENV_KEY}_AR"
    (
      export "$CC_VAR=$TOOLCHAIN_BIN/${CLANG_PREFIX}-clang"
      export "$CXX_VAR=$TOOLCHAIN_BIN/${CLANG_PREFIX}-clang++"
      export "$AR_VAR=$TOOLCHAIN_BIN/llvm-ar"
      export "$RANLIB_VAR=$TOOLCHAIN_BIN/llvm-ranlib"
      export "$LINKER_VAR=$TOOLCHAIN_BIN/${CLANG_PREFIX}-clang"
      export "$TARGET_AR_VAR=$TOOLCHAIN_BIN/llvm-ar"
      nola_cargo build \
        --locked \
        --manifest-path "$ROOT/Cargo.toml" \
        --profile "$PROFILE" \
        --target "$TRIPLE" \
        -p nola-app \
        --lib
    )
  fi

  SRC_LIB="$CARGO_TARGET_DIR/$TRIPLE/$OUTPUT_PROFILE/libnola_app.a"
  [[ -f "$SRC_LIB" ]] || { echo "error: expected Rust staticlib at $SRC_LIB" >&2; exit 2; }
  mkdir -p "$OUTPUT_DIR/$abi"
  DST_LIB="$OUTPUT_DIR/$abi/libnola_core.a"
  if [[ ! -f "$DST_LIB" ]] || ! cmp -s "$SRC_LIB" "$DST_LIB"; then
    cp -f "$SRC_LIB" "$DST_LIB"
  fi
done
