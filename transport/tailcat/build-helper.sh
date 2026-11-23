#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# Build the optional nola-tailcat stdio helper for the current platform and
# print its canonical path and SHA-256 digest for --tailcat-helper configuration.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

GO_BIN="${GO:-go}"
if ! command -v "$GO_BIN" >/dev/null 2>&1; then
  echo "go toolchain not found; install Go or set GO=/absolute/path/to/go" >&2
  exit 1
fi

EXT=""
case "$(uname -s 2>/dev/null || echo unknown)" in
  MINGW*|MSYS*|CYGWIN*) EXT=".exe" ;;
esac

OUTPUT="${1:-$SCRIPT_DIR/build/nola-tailcat${EXT}}"
mkdir -p "$(dirname "$OUTPUT")"

export GOENV=off
export GOTOOLCHAIN=local
"$GO_BIN" build -trimpath -tags=ts_omit_ssh -o "$OUTPUT" ./cmd/nola-tailcat

ABS_OUTPUT="$(cd "$(dirname "$OUTPUT")" && pwd)/$(basename "$OUTPUT")"
if command -v shasum >/dev/null 2>&1; then
  DIGEST="$(shasum -a 256 "$ABS_OUTPUT" | awk '{print $1}')"
elif command -v sha256sum >/dev/null 2>&1; then
  DIGEST="$(sha256sum "$ABS_OUTPUT" | awk '{print $1}')"
else
  echo "Neither shasum nor sha256sum is available to compute SHA-256" >&2
  exit 1
fi

echo "Built nola-tailcat helper:"
echo "  path:   $ABS_OUTPUT"
echo "  sha256: $DIGEST"
echo ""
echo "Host example:"
echo "  nola server --tailcat-ingress --tailcat-helper \"$ABS_OUTPUT\" --tailcat-helper-sha256 \"$DIGEST\""
