#!/usr/bin/env bash
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
  -p nola_cli \
  --bin nola \
  --bin nola-agent-host

if [[ "$OUTPUT_DIR" != "$ARTIFACT_DIR" ]]; then
  install -m 755 "$ARTIFACT_DIR/nola" "$OUTPUT_DIR/nola"
  install -m 755 "$ARTIFACT_DIR/nola-agent-host" "$OUTPUT_DIR/nola-agent-host"
fi

for required in \
  "$OUTPUT_DIR/nola" \
  "$OUTPUT_DIR/nola-agent-host"; do
  [[ -f "$required" ]] || { echo "error: incomplete Nola runtime: $required" >&2; exit 3; }
done

printf 'Nola CLI/Web runtime: %s\n' "$OUTPUT_DIR"
