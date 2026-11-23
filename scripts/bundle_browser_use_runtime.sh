#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# Build a relocatable Browser Use execution runtime; no credentials or model setup.
set -euo pipefail
SOURCE="${NOLA_BROWSER_USE_SOURCE:-$(cd "$(dirname "$0")/.." && pwd)/apps/web/browser_use_runtime}"
DESTINATION="${1:?Pass the browser_use output directory}"
SHARED_CACHE_ROOT="${HOME:+${HOME}/Library/Caches/NolaBuild}"
CACHE="${NOLA_BROWSER_USE_CACHE:-${SHARED_CACHE_ROOT:+${SHARED_CACHE_ROOT}/browser-use-runtime}}"
CACHE="${CACHE:-${DERIVED_FILE_DIR:-${TMPDIR:-/tmp}/nola-build}/browser-use-runtime}"

clone_tree() {
    cp -cR "$1" "$2" 2>/dev/null || cp -R "$1" "$2"
}

mkdir -p "$CACHE"
SIGN_KEY="${CODE_SIGNING_ALLOWED:-NO}:${CONFIGURATION:-Debug}:${NOLA_MAC_DISTRIBUTION_IDENTITY:--}:$(uname -m)"
SNAPSHOT_HASH="$(printf '%s\n' "$SIGN_KEY" | cat "$SOURCE/requirements.lock" - | /usr/bin/shasum -a 256 | awk '{print $1}')"
SNAPSHOT_DIR="$CACHE/snapshots/$SNAPSHOT_HASH"

if [[ ! -f "$SNAPSHOT_DIR/.ready" ]]; then
    UV_BIN="${NOLA_UV_BIN:-$(command -v uv || true)}"
    if [[ -z "$UV_BIN" ]]; then
        for candidate in /opt/homebrew/bin/uv /usr/local/bin/uv; do
            if [[ -x "$candidate" ]]; then UV_BIN="$candidate"; break; fi
        done
    fi
    [[ -x "$UV_BIN" ]] || { echo 'error: Browser Use bundling requires uv; install uv or set NOLA_UV_BIN.' >&2; exit 1; }
    export UV_PYTHON_INSTALL_DIR="$CACHE/interpreters"
    "$UV_BIN" python install 3.12.12
    PYTHON_BIN="$("$UV_BIN" python find --managed-python --resolve-links 3.12.12)"
    PYTHON_ROOT="$(cd "$(dirname "$PYTHON_BIN")/.." && pwd)"
    mkdir -p "$CACHE/snapshots"
    STAGING="$(mktemp -d "$CACHE/snapshots/package.XXXXXX")"
    trap 'rm -rf "$STAGING"' EXIT
    clone_tree "$PYTHON_ROOT" "$STAGING/python"
    "$UV_BIN" pip install --python "$STAGING/python/bin/python3" --target "$STAGING/site-packages" \
        --no-deps --requirements "$SOURCE/requirements.lock"
    PYTHONNOUSERSITE=1 PYTHONPATH="$STAGING/site-packages" "$STAGING/python/bin/python3" -c \
        'from importlib.metadata import version; assert version("browser-use") == "1.0.2"; assert version("browser-harness") == "0.1.13"'
    if [[ "${CODE_SIGNING_ALLOWED:-NO}" == YES && "$(uname -s)" == Darwin ]]; then
        . "$(dirname "$0")/lib/install_mac_executable.sh"
        # Python extensions are loaded by the bundled child, not the application.
        # Sign each Mach-O in the relocated runtime, as for the existing Node child.
        while IFS= read -r -d '' binary; do
            if /usr/bin/file -b "$binary" | /usr/bin/grep -q 'Mach-O'; then
                nola_sign_embedded_binary "$binary"
            fi
        done < <(find "$STAGING" -type f \( -name '*.so' -o -name '*.dylib' -o -name 'python3.12' \) -print0)
    fi
    touch "$STAGING/.ready"
    rm -rf "$SNAPSHOT_DIR"
    mv "$STAGING" "$SNAPSHOT_DIR"
    trap - EXIT
fi

mkdir -p "$(dirname "$DESTINATION")"
rm -rf "$DESTINATION"
clone_tree "$SNAPSHOT_DIR" "$DESTINATION"
rm -f "$DESTINATION/.ready"
cp "$SOURCE/worker.py" "$SOURCE/requirements.in" "$SOURCE/requirements.lock" "$DESTINATION/"
