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
LIB_NAME="libnola_core.a"
BUILD_DIR="$ROOT/target/release"
LIB_PATH="$BUILD_DIR/libnola_mobile_bindings.a"

if [[ "$TARGET" == "darwin" ]]; then
    # Build only the two requested slices; do not also build an unused host copy.
    for triple in x86_64-apple-darwin aarch64-apple-darwin; do
        CARGO_TARGET_DIR="$ROOT/target" nola_cargo rustc --locked --manifest-path "$BINDINGS_MANIFEST" --lib --crate-type staticlib --release --target "$triple"
    done
    mkdir -p "$BUILD_DIR"
    MERGED="$BUILD_DIR/libnola_core.merged.a"
    lipo -create -output "$MERGED" \
        "$ROOT/target/x86_64-apple-darwin/release/libnola_mobile_bindings.a" \
        "$ROOT/target/aarch64-apple-darwin/release/libnola_mobile_bindings.a"
    if ! cmp -s "$MERGED" "$LIB_PATH"; then mv "$MERGED" "$LIB_PATH"; else rm "$MERGED"; fi
else
    CARGO_TARGET_DIR="$ROOT/target" nola_cargo rustc --locked --manifest-path "$BINDINGS_MANIFEST" --lib --crate-type staticlib --release --target "$TARGET"
    LIB_PATH="$ROOT/target/$TARGET/release/libnola_mobile_bindings.a"
fi

if [[ ! -f "$LIB_PATH" ]]; then
    echo "Library $LIB_PATH not found. Check cargo build output." >&2
    exit 1
fi

mkdir -p "$OUT_DIR"
if ! cmp -s "$LIB_PATH" "$OUT_DIR/$LIB_NAME"; then cp "$LIB_PATH" "$OUT_DIR/$LIB_NAME"; fi
if ! cmp -s "$ROOT/swift_shims/nola_core.h" "$OUT_DIR/nola_core.h"; then cp "$ROOT/swift_shims/nola_core.h" "$OUT_DIR/"; fi

echo "Copied $LIB_NAME and header to $OUT_DIR"
