#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
set -euo pipefail
. "${SRCROOT}/scripts/lib/install_mac_executable.sh"
SOURCE="${BUILT_PRODUCTS_DIR}/NolaRuntime"
MACOS="${TARGET_BUILD_DIR}/${CONTENTS_FOLDER_PATH}/MacOS"
RESOURCES="${TARGET_BUILD_DIR}/${UNLOCALIZED_RESOURCES_FOLDER_PATH}"
AGENTS="${TARGET_BUILD_DIR}/${CONTENTS_FOLDER_PATH}/Library/LaunchAgents"
mkdir -p "$MACOS" "$RESOURCES" "$AGENTS"
nola_install_mac_executable "$SOURCE/nola" "$MACOS/nola" "${CODE_SIGNING_ALLOWED:-NO}"
nola_install_mac_executable "${BUILT_PRODUCTS_DIR}/nola-mlx-runtime" "$MACOS/nola-mlx-runtime" "${CODE_SIGNING_ALLOWED:-NO}"
TAILCAT_DIR="${SRCROOT}/transport/tailcat"
TAILCAT_BIN="$SOURCE/nola-tailcat"
GO_BIN="$(command -v go || true)"
if [[ -z "$GO_BIN" ]]; then
    for candidate in /opt/homebrew/bin/go /usr/local/go/bin/go; do
        if [[ -x "$candidate" ]]; then
            GO_BIN="$candidate"
            break
        fi
    done
fi
if [[ -n "$GO_BIN" && -f "$TAILCAT_DIR/go.mod" ]]; then
    mkdir -p "$SOURCE"
    RAW_ARCH="${ARCHS%% *}"
    if [[ -z "$RAW_ARCH" || "$RAW_ARCH" == "undefined_arch" ]]; then
        RAW_ARCH="$(uname -m)"
    fi
    case "$RAW_ARCH" in
        arm64|aarch64) TARGET_GOARCH="arm64" ;;
        x86_64|amd64) TARGET_GOARCH="amd64" ;;
        *) TARGET_GOARCH="$(uname -m | sed -e 's/aarch64/arm64/' -e 's/x86_64/amd64/')" ;;
    esac
    NEED_TAILCAT_BUILD=0
    if [[ ! -x "$TAILCAT_BIN" ]]; then
        NEED_TAILCAT_BUILD=1
    elif [[ -n "$(find "$TAILCAT_DIR" \( -name '*.go' -o -name 'go.mod' -o -name 'go.sum' \) -newer "$TAILCAT_BIN" -print -quit 2>/dev/null)" ]]; then
        NEED_TAILCAT_BUILD=1
    fi
    if [[ "$NEED_TAILCAT_BUILD" == 1 ]]; then
        (cd "$TAILCAT_DIR" && GOOS=darwin GOARCH="$TARGET_GOARCH" CGO_ENABLED=0 "$GO_BIN" build -trimpath -tags=ts_omit_ssh -o "$TAILCAT_BIN" ./cmd/nola-tailcat)
    fi
fi
if [[ -x "$TAILCAT_BIN" ]]; then
    nola_install_mac_executable "$TAILCAT_BIN" "$MACOS/nola-tailcat" "${CODE_SIGNING_ALLOWED:-NO}"
    /usr/bin/shasum -a 256 "$MACOS/nola-tailcat" | awk '{print $1}' > "$RESOURCES/nola-tailcat.sha256"
    chmod 644 "$RESOURCES/nola-tailcat.sha256"
else
    rm -f "$RESOURCES/nola-tailcat.sha256"
    echo "note: skipping bundled nola-tailcat (Go toolchain not found and no prebuilt helper at $TAILCAT_BIN)" >&2
fi
rm -rf "$RESOURCES/mlx-swift_Cmlx.bundle"
# Archive builds expose this SwiftPM product through a link into DerivedData.
cp -cRH "${BUILT_PRODUCTS_DIR}/mlx-swift_Cmlx.bundle" "$RESOURCES/mlx-swift_Cmlx.bundle" 2>/dev/null || \
    cp -RH "${BUILT_PRODUCTS_DIR}/mlx-swift_Cmlx.bundle" "$RESOURCES/mlx-swift_Cmlx.bundle"
test -s "$RESOURCES/mlx-swift_Cmlx.bundle/Contents/Resources/default.metallib"
cp "$SOURCE/com.gaojc.Nola.background-scheduler.plist" "$AGENTS/com.gaojc.Nola.background-scheduler.plist"
chmod 644 "$AGENTS/com.gaojc.Nola.background-scheduler.plist"
