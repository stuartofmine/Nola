#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# File Overview:
# - Purpose: Builds the local Svelte client and Rust Host, then serves Web.
# - Interfaces: WEB_HOST, WEB_PORT, and CLI Web arguments.
# - APIs: none
set -euo pipefail
ROOT="$(cd "$(dirname "$0")" && pwd)"
cd "$ROOT"

npm --prefix web/frontend ci
npm --prefix web/frontend run build
. "$ROOT/scripts/lib/rust_build_cache.sh"
nola_rust_build_cache_init "${CARGO:-cargo}"
nola_cargo build --locked -p nola_cli -p nola_service --bins

exec "${CARGO_TARGET_DIR:-$ROOT/target}/debug/nola" web \
  --host "${WEB_HOST:-127.0.0.1}" --port "${WEB_PORT:-5001}" \
  --web-root "$ROOT/web" "$@"
