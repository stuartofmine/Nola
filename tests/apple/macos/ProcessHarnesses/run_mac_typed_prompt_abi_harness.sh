#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
BUILD_DIR=$(mktemp -d "${TMPDIR:-/tmp}/nola-mac-typed-prompt.XXXXXX")
trap 'rm -rf "$BUILD_DIR"' EXIT INT TERM
CORE_LIB="$ROOT/core/target/aarch64-apple-darwin/release"

cp "$ROOT/tests/apple/macos/ProcessHarnesses/MacTypedPromptABIHarnessMain.swift.fixture" \
    "$BUILD_DIR/MacTypedPromptABIHarnessMain.swift"
xcrun swiftc \
    -I "$ROOT/core/swift_shims" \
    -L "$CORE_LIB" \
    -lnola_core \
    "$ROOT/apps/macos/Services/MacCorePromptComposer.swift" \
    "$BUILD_DIR/MacTypedPromptABIHarnessMain.swift" \
    -o "$BUILD_DIR/mac-typed-prompt-abi-harness"
DYLD_LIBRARY_PATH="$CORE_LIB${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}" \
    "$BUILD_DIR/mac-typed-prompt-abi-harness"
