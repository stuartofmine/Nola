#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# File Overview:
# - Purpose: Automates operational steps for start in the backend infrastructure.
# - Interfaces:
#   - Internal implementation details only.
# - APIs:
#   - none
#


set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
WORKER_DIR="$SCRIPT_DIR/worker"
NPM_BIN="${NPM_BIN:-npm}"
NODE_AUTO_INSTALL_DEPS="${NODE_AUTO_INSTALL_DEPS:-1}"
CMD="${1:-start}"
if [ "$#" -gt 0 ]; then
  shift
fi

if [ ! -d "$WORKER_DIR" ]; then
  echo "ERROR: Worker directory not found: $WORKER_DIR" >&2
  exit 1
fi
if ! command -v "$NPM_BIN" >/dev/null 2>&1; then
  echo "ERROR: npm is required" >&2
  exit 1
fi

cd "$WORKER_DIR"

# Avoid Safari OAuth popups on macOS when tools try browser auth.
if [ "${BROWSER:-}" = "" ] && [ "$(uname -s 2>/dev/null || echo)" = "Darwin" ]; then
  export BROWSER="Google Chrome"
fi

if [ "$CMD" = "install" ]; then
  exec "$NPM_BIN" install --silent
fi

if [ "$NODE_AUTO_INSTALL_DEPS" != "0" ]; then
  "$NPM_BIN" install --silent
fi

case "$CMD" in
  start|dev)
    exec "$NPM_BIN" run dev -- "$@"
    ;;
  deploy)
    exec "$NPM_BIN" run deploy -- "$@"
    ;;
  migrate:remote)
    if [ -z "${CLOUDFLARE_API_TOKEN:-${CF_API_TOKEN:-}}" ]; then
      echo "ERROR: migrate:remote requires CLOUDFLARE_API_TOKEN (or CF_API_TOKEN)." >&2
      echo "Set API token auth to avoid browser OAuth opening Safari." >&2
      exit 2
    fi
    exec "$NPM_BIN" run "$CMD" -- "$@"
    ;;
  typecheck|migrate:local)
    exec "$NPM_BIN" run "$CMD" -- "$@"
    ;;
  *)
    exec "$NPM_BIN" run "$CMD" -- "$@"
    ;;
esac
