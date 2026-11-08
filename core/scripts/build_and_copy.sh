#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# File Overview:
# - Purpose: Builds the Rust core and copies its static library and header.
# - Interfaces:
#   - CLI flags: --target and --out-dir.
# - APIs:
#   - none
#
set -euo pipefail

# Usage: ./build_and_copy.sh [--target <target triple>] [--out-dir <path>]

ROOT="$(cd "$(dirname "$0")/.." && pwd -P)"
. "$ROOT/../scripts/lib/rust_build_cache.sh"
nola_rust_build_cache_init "${CARGO:-cargo}"
BINDINGS_MANIFEST="$ROOT/nola-mobile-bindings/Cargo.toml"
TARGET="darwin"
OUT_DIR="$ROOT/../Frameworks"

if [[ "${1:-}" == "--target" ]]; then
    TARGET="$2"
    shift 2
fi
if [[ "${1:-}" == "--out-dir" ]]; then
    OUT_DIR="$2"
    shift 2
fi

echo "Building nola mobile bindings for $TARGET..."
CARGO_TARGET_DIR="$ROOT/target" nola_cargo build --manifest-path "$BINDINGS_MANIFEST" --release

LIB_NAME="libnola_core.a"
BUILD_DIR="$ROOT/target/release"
LIB_PATH="$BUILD_DIR/libnola_mobile_bindings.a"

if [[ "$TARGET" == "darwin" ]]; then
    CARGO_TARGET_DIR="$ROOT/target" nola_cargo build --manifest-path "$BINDINGS_MANIFEST" --release --target x86_64-apple-darwin
    CARGO_TARGET_DIR="$ROOT/target" nola_cargo build --manifest-path "$BINDINGS_MANIFEST" --release --target aarch64-apple-darwin || true

    LIB_X86="$ROOT/target/x86_64-apple-darwin/release/libnola_mobile_bindings.a"
    LIB_ARM="$ROOT/target/aarch64-apple-darwin/release/libnola_mobile_bindings.a"
    if [[ -f "$LIB_X86" && -f "$LIB_ARM" ]]; then
        lipo -create -output "$LIB_PATH" "$LIB_X86" "$LIB_ARM"
    fi
fi

if [[ ! -f "$LIB_PATH" ]]; then
    echo "Library $LIB_PATH not found. Check cargo build output." >&2
    exit 1
fi

mkdir -p "$OUT_DIR"
cp "$LIB_PATH" "$OUT_DIR/$LIB_NAME"
cp "$ROOT/swift_shims/nola_core.h" "$OUT_DIR/"

echo "Copied $LIB_NAME and header to $OUT_DIR"
