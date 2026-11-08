#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
set -euo pipefail
. "${SRCROOT}/scripts/lib/install_mac_executable.sh"
SOURCE="${BUILT_PRODUCTS_DIR}/NolaRuntime"
MACOS="${TARGET_BUILD_DIR}/${CONTENTS_FOLDER_PATH}/MacOS"
RESOURCES="${TARGET_BUILD_DIR}/${UNLOCALIZED_RESOURCES_FOLDER_PATH}"
AGENTS="${TARGET_BUILD_DIR}/${CONTENTS_FOLDER_PATH}/Library/LaunchAgents"
mkdir -p "$MACOS" "$RESOURCES" "$AGENTS"
nola_install_mac_executable "$SOURCE/nola-agent-host" "$MACOS/nola-agent-host" "${CODE_SIGNING_ALLOWED:-NO}"
nola_install_mac_executable "$SOURCE/nola-service" "$MACOS/nola-service" "${CODE_SIGNING_ALLOWED:-NO}"
nola_install_mac_executable "${BUILT_PRODUCTS_DIR}/nola-mlx-runtime" "$MACOS/nola-mlx-runtime" "${CODE_SIGNING_ALLOWED:-NO}"
rm -rf "$RESOURCES/mlx-swift_Cmlx.bundle"
cp -R "${BUILT_PRODUCTS_DIR}/mlx-swift_Cmlx.bundle" "$RESOURCES/mlx-swift_Cmlx.bundle"
cp "$SOURCE/com.gaojc.Nola.background-scheduler.plist" "$AGENTS/com.gaojc.Nola.background-scheduler.plist"
chmod 644 "$AGENTS/com.gaojc.Nola.background-scheduler.plist"
