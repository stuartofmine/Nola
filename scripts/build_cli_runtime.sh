#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
set -euo pipefail
. "$(cd "$(dirname "$0")" && pwd -P)/lib/rust_build_cache.sh"
nola_rust_build_cache_init "${CARGO:-cargo}"

ROOT="${NOLA_REPOSITORY_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
PROFILE="${NOLA_BUILD_PROFILE:-release}"
CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"

case "$PROFILE" in
  dev) OUTPUT_PROFILE=debug ;;
  *) OUTPUT_PROFILE="$PROFILE" ;;
esac

OUTPUT_DIR="${NOLA_RUNTIME_OUTPUT_DIR:-$CARGO_TARGET_DIR/$OUTPUT_PROFILE}"
ARTIFACT_DIR="$CARGO_TARGET_DIR/$OUTPUT_PROFILE"
mkdir -p "$OUTPUT_DIR"

CARGO_TARGET_DIR="$CARGO_TARGET_DIR" nola_cargo build \
  --locked \
  --manifest-path "$ROOT/Cargo.toml" \
  --profile "$PROFILE" \
  -p nola-app \
  --bin nola

if [[ "$OUTPUT_DIR" != "$ARTIFACT_DIR" ]]; then
  install -m 755 "$ARTIFACT_DIR/nola" "$OUTPUT_DIR/nola"
fi

[[ -f "$OUTPUT_DIR/nola" ]] || { echo "error: incomplete Nola runtime: $OUTPUT_DIR/nola" >&2; exit 3; }

printf 'Nola CLI/Web runtime: %s\n' "$OUTPUT_DIR"
