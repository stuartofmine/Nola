#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# File Overview:
# - Purpose: Automates operational steps for install host in the web client.
# - Interfaces:
#   - Internal implementation details only.
# - APIs:
#   - none
#
set -euo pipefail

DEFAULT_EXTENSION_ID="abebajkkopomanodfidanihmfklgibbo"
EXTENSION_ID="$DEFAULT_EXTENSION_ID"
SERVER_URL="http://127.0.0.1:5001"
TOKEN="${NOLA_NATIVE_BROWSER_TOKEN:-}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --extension-id)
      EXTENSION_ID="${2:-}"
      shift 2
      ;;
    --server-url)
      SERVER_URL="${2:-}"
      shift 2
      ;;
    --token)
      TOKEN="${2:-}"
      shift 2
      ;;
    *)
      echo "Unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PY_HOST_PATH="$SCRIPT_DIR/host/nola_native_browser_host.py"
chmod +x "$PY_HOST_PATH"

PYTHON_BIN="$(command -v python3 || true)"
if [[ -z "$PYTHON_BIN" ]]; then
  echo "python3 not found. Install Python 3 before installing the native host." >&2
  exit 2
fi

if [[ -z "$TOKEN" ]]; then
  TOKEN="$("$PYTHON_BIN" - <<'PY'
import secrets
print(secrets.token_urlsafe(32))
PY
)"
fi

if [[ -z "$EXTENSION_ID" ]]; then
  echo "Usage: $0 [--extension-id <chrome-extension-id>] [--server-url http://127.0.0.1:5001] [--token TOKEN]" >&2
  echo "Defaults to the packaged CRX extension ID: $DEFAULT_EXTENSION_ID" >&2
  exit 2
fi
if [[ ! "$EXTENSION_ID" =~ ^[a-p]{32}$ ]]; then
  echo "Invalid Chrome extension ID: $EXTENSION_ID" >&2
  echo "Expected the 32-character ID shown in chrome://extensions." >&2
  exit 2
fi
if [[ "$SERVER_URL" != http://* && "$SERVER_URL" != https://* ]]; then
  echo "Invalid server URL: $SERVER_URL" >&2
  echo "Expected http:// or https://." >&2
  exit 2
fi

mkdir -p "$HOME/.nola"
if [[ -f "$HOME/.nola/native_browser_host.json" ]]; then
  cp "$HOME/.nola/native_browser_host.json" "$HOME/.nola/native_browser_host.json.bak"
fi
NOLA_NATIVE_SERVER_URL="$SERVER_URL" \
NOLA_NATIVE_TOKEN="$TOKEN" \
"$PYTHON_BIN" - <<'PY'
import json
import os
from pathlib import Path
config = {
    "server_url": os.environ["NOLA_NATIVE_SERVER_URL"].rstrip("/"),
    "token": os.environ["NOLA_NATIVE_TOKEN"],
    "host_id": "chrome-default",
    "local_user_id": "cli-local-user",
}
path = Path.home() / ".nola" / "native_browser_host.json"
path.write_text(json.dumps(config, indent=2), encoding="utf-8")
PY
chmod 600 "$HOME/.nola/native_browser_host.json"

LAUNCHER_PATH="$HOME/.nola/nola_native_browser_host_launcher.sh"
NOLA_NATIVE_LAUNCHER_PATH="$LAUNCHER_PATH" \
NOLA_NATIVE_PYTHON_BIN="$PYTHON_BIN" \
NOLA_NATIVE_PY_HOST_PATH="$PY_HOST_PATH" \
"$PYTHON_BIN" - <<'PY'
import os
import shlex
from pathlib import Path

launcher_path = Path(os.environ["NOLA_NATIVE_LAUNCHER_PATH"])
python_bin = os.environ["NOLA_NATIVE_PYTHON_BIN"]
host_path = os.environ["NOLA_NATIVE_PY_HOST_PATH"]
launcher_path.write_text(
    "\n".join(
        [
            "#!/bin/sh",
            'LOG="$HOME/.nola/native_browser_host_launcher.log"',
            "{",
            "  printf '[nola-native-browser-launcher] %s starting python=%s host=%s\\n' "
            "\"$(date -u '+%Y-%m-%dT%H:%M:%SZ')\" "
            f"{shlex.quote(python_bin)} {shlex.quote(host_path)}",
            '} >> "$LOG" 2>&1',
            'export NOLA_NATIVE_BROWSER_CONFIG="$HOME/.nola/native_browser_host.json"',
            f"exec {shlex.quote(python_bin)} {shlex.quote(host_path)} 2>> \"$LOG\"",
            "",
        ]
    ),
    encoding="utf-8",
)
PY
chmod 755 "$LAUNCHER_PATH"

MANIFEST_DIRS=()
case "$(uname -s)" in
  Darwin)
    MANIFEST_DIRS+=(
      "$HOME/Library/Application Support/Google/Chrome/NativeMessagingHosts"
      "$HOME/Library/Application Support/Chromium/NativeMessagingHosts"
      "$HOME/Library/Application Support/Microsoft Edge/NativeMessagingHosts"
      "$HOME/Library/Application Support/BraveSoftware/Brave-Browser/NativeMessagingHosts"
    )
    ;;
  Linux)
    MANIFEST_DIRS+=(
      "$HOME/.config/google-chrome/NativeMessagingHosts"
      "$HOME/.config/chromium/NativeMessagingHosts"
      "$HOME/.config/microsoft-edge/NativeMessagingHosts"
      "$HOME/.config/BraveSoftware/Brave-Browser/NativeMessagingHosts"
    )
    ;;
  *)
    echo "Unsupported OS for this installer. Create the Chrome native messaging manifest manually." >&2
    exit 2
    ;;
esac

for MANIFEST_DIR in "${MANIFEST_DIRS[@]}"; do
  mkdir -p "$MANIFEST_DIR"
  if [[ -f "$MANIFEST_DIR/com.nola.native_browser.json" ]]; then
    cp "$MANIFEST_DIR/com.nola.native_browser.json" "$MANIFEST_DIR/com.nola.native_browser.json.bak"
  fi
  NOLA_NATIVE_MANIFEST_DIR="$MANIFEST_DIR" \
  NOLA_NATIVE_LAUNCHER_PATH="$LAUNCHER_PATH" \
  NOLA_NATIVE_EXTENSION_ID="$EXTENSION_ID" \
  "$PYTHON_BIN" - <<'PY'
import json
import os
from pathlib import Path
manifest = {
    "name": "com.nola.native_browser",
    "description": "Nola native browser bridge",
    "path": os.environ["NOLA_NATIVE_LAUNCHER_PATH"],
    "type": "stdio",
    "allowed_origins": [f"chrome-extension://{os.environ['NOLA_NATIVE_EXTENSION_ID']}/"],
}
Path(os.environ["NOLA_NATIVE_MANIFEST_DIR"], "com.nola.native_browser.json").write_text(
    json.dumps(manifest, indent=2),
    encoding="utf-8",
)
PY
  echo "Installed native host manifest: $MANIFEST_DIR/com.nola.native_browser.json"
done

CRX_PATH="$SCRIPT_DIR/extension.crx"
echo "Wrote host config: $HOME/.nola/native_browser_host.json"
echo "Wrote host launcher: $LAUNCHER_PATH"
echo "Configured Chrome extension ID: $EXTENSION_ID"
if [[ -f "$CRX_PATH" ]]; then
  echo "Packaged extension CRX: $CRX_PATH"
  echo "If Chrome rejects the local CRX with CRX_REQUIRED_PROOF_MISSING, load this unpacked directory instead:"
  echo "$SCRIPT_DIR/extension"
  echo "The unpacked manifest includes the same public key, so it keeps extension ID: $EXTENSION_ID"
fi
echo "The CLI/Web Host and Native Messaging process now discover this configuration automatically."
