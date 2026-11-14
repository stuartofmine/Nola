#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# File Overview:
# - Purpose: Automates operational steps for integrate xcframework in the project module.
# - Interfaces:
#   - Internal implementation details only.
# - APIs:
#   - none
#
set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd -P)"
CORE_ROOT="$PROJECT_ROOT/core"
XCFRAMEWORK="$CORE_ROOT/target_xcframework/nola_core.xcframework"
DEST_FRAMEWORK_DIR="$PROJECT_ROOT/Frameworks"

if [ ! -d "$XCFRAMEWORK" ]; then
  echo "XCFramework not found. Run core/scripts/build_xcframework.sh first." >&2
  exit 2
fi

mkdir -p "$DEST_FRAMEWORK_DIR"
rm -rf "$DEST_FRAMEWORK_DIR/nola_core.xcframework"
cp -R "$XCFRAMEWORK" "$DEST_FRAMEWORK_DIR/"

echo "Copied nola_core.xcframework to $DEST_FRAMEWORK_DIR"

echo "Add the xcframework to your Xcode project:"
echo "  - Open apple/Nola.xcodeproj and drag $DEST_FRAMEWORK_DIR/nola_core.xcframework into Frameworks (or to the project root)."
echo "  - The project links nola_core.xcframework as a static framework; keep Embed set to 'Do Not Embed'."
echo "  - The bundled module.modulemap exposes the Swift module as NolaCore."

exit 0
