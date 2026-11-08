#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
BUILD_DIR=$(mktemp -d "${TMPDIR:-/tmp}/nola-mac-source-projection.XXXXXX")
trap 'rm -rf "$BUILD_DIR"' EXIT INT TERM

cp "$ROOT/NolaMacTests/ProcessHarnesses/MacCoreSourceProjectionHarnessMain.swift.fixture" \
    "$BUILD_DIR/MacCoreSourceProjectionHarnessMain.swift"
xcrun swiftc \
    "$ROOT/NolaMac/Models/MacCoreSourceProjection.swift" \
    "$BUILD_DIR/MacCoreSourceProjectionHarnessMain.swift" \
    -o "$BUILD_DIR/mac-core-source-projection-harness"
"$BUILD_DIR/mac-core-source-projection-harness"
