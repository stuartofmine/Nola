#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# File Overview:
# - Purpose: Automates operational steps for verify stream in the backend infrastructure.
# - Interfaces:
#   - cleanup
# - APIs:
#   - none
#
# Quick test: verify the streaming endpoint actually streams SSE events.
# This gateway requires X-Client-UUID for request attribution.
#
# Usage:
#   ./verify_stream.sh https://darkblock.cn:16384/nolaServer <X_CLIENT_UUID>
#
# Example:
#   ./verify_stream.sh https://darkblock.cn:16384/nolaServer fda6d625993b4553b35bf9ecd8b983b3

set -e

ENDPOINT="${1:-http://127.0.0.1:8000}"
CLIENT_UUID="${2:-}"

# Optional: pass extra curl flags/headers via env var, e.g.
#   EXTRA_CURL_HEADERS='-H "X-Mock-Hang: 1"' ./verify_stream.sh http://127.0.0.1:8000 00000000-0000-0000-0000-000000000000
EXTRA_CURL_HEADERS=${EXTRA_CURL_HEADERS:-""}

# How long to wait for the stream before curl aborts (seconds).
# If upstream/gateway hangs mid-stream, you want this long enough to observe retry/fallback behavior.
STREAM_MAX_TIME=${STREAM_MAX_TIME:-120}

# Optional: override request payload (JSON string). Defaults to a tiny prompt.
# Example:
#   PAYLOAD='{"model":"deepseek-ai/DeepSeek-V3.2","messages":[{"role":"user","content":"hello"}],"stream":true,"stream_options":{"include_usage":true}}' \
#   ./verify_stream.sh http://127.0.0.1:8000 <uuid>
PAYLOAD=${PAYLOAD:-'{"model":"deepseek-ai/DeepSeek-V3.2","messages":[{"role":"user","content":"写个bfs"}],"stream":true,"stream_options":{"include_usage":true}}'}

if [ -z "$CLIENT_UUID" ]; then
  echo "Usage: $0 <endpoint> <x_client_uuid>" >&2
  echo "Example: $0 https://darkblock.cn:16384/nolaServer fda6d625993b4553b35bf9ecd8b983b3" >&2
  exit 1
fi

echo "Testing streaming endpoint: $ENDPOINT/v1/chat/completions"
echo "Payload: $PAYLOAD"
echo "X-Client-UUID: $CLIENT_UUID"
echo ""
echo "--- Stream response (headers + first 80 lines) ---"

OUT="$(mktemp -t nola_verify_stream_XXXXXX.txt)"
cleanup() { rm -f "$OUT"; }
trap cleanup EXIT

# Capture a short burst so we can count events and show a sample.
curl -sS -i -N --no-buffer --max-time "$STREAM_MAX_TIME" \
  --http1.1 \
  -H "Content-Type: application/json" \
  -H "Accept: text/event-stream" \
  -H "Accept-Encoding: identity" \
  -H "X-Client-UUID: $CLIENT_UUID" \
  $EXTRA_CURL_HEADERS \
  -d "$PAYLOAD" \
  "$ENDPOINT/v1/chat/completions" > "$OUT" || true

sed -n '1,80p' "$OUT"

echo ""
echo "--- Gateway diagnostics headers (if present) ---"
grep -iE '^(x-gateway-build-id|x-gateway-sse-rechunk|x-gateway-stream-read-timeout|x-accel-buffering):' "$OUT" || true

echo ""
echo "--- SSE event count (lines starting with 'data:') ---"
grep -c '^data:' "$OUT" || true

echo ""
echo "--- First 20 SSE data lines (for quick sanity) ---"
grep '^data:' "$OUT" | head -n 20 || true

echo ""
echo "--- Last 20 SSE data lines (to see if stream progresses) ---"
grep '^data:' "$OUT" | tail -n 20 || true

echo ""
echo "--- Completion markers (DONE / finish_reason) ---"
grep -E 'data: \[DONE\]|"finish_reason"\s*:\s*"(stop|length|tool_calls|content_filter)"' "$OUT" | tail -n 10 || true

echo ""
echo "--- SSE integrity checks (strict, best-effort) ---"

# 1) Basic response headers
STATUS_LINE="$(grep -m1 -E '^HTTP/' "$OUT" || true)"
CT_LINE="$(grep -im1 -E '^content-type:' "$OUT" || true)"
echo "HTTP: ${STATUS_LINE:-"(missing)"}"
echo "Content-Type: ${CT_LINE:-"(missing)"}"

# 2) Check we ended with a complete SSE boundary and [DONE]
#    (Broken streams often end with partial JSON without \n\n.)
if tail -c 2 "$OUT" | grep -q $'\n\n'; then
  echo "SSE boundary at EOF: yes"
else
  echo "SSE boundary at EOF: NO (possible truncated last event)"
fi

if grep -q '^data: \[DONE\]' "$OUT"; then
  echo "Saw [DONE]: yes"
else
  echo "Saw [DONE]: NO"
fi

# 3) Parse SSE data lines for JSON validity and usage chunk presence.
#    We ignore [DONE] and tolerate comment/empty lines.
python3 - "$OUT" <<'PY' || true
import json
import sys

path = sys.argv[1]

total = 0
done = 0
json_ok = 0
json_fail = 0
usage_chunks = 0
choice_chunks = 0
empty_choices_usage = 0

def is_json_obj(s: str) -> bool:
  s = s.strip()
  if not s:
    return False
  if not (s.startswith('{') and s.endswith('}')):
    return False
  return True

with open(path, 'rb') as f:
  for raw in f.read().splitlines():
    try:
      line = raw.decode('utf-8', errors='ignore')
    except Exception:
      continue
    if not line.startswith('data:'):
      continue
    data = line[len('data:'):].strip()
    if not data:
      continue
    total += 1
    if data == '[DONE]':
      done += 1
      continue
    # Token marker is not SSE JSON; skip
    if data.startswith('__TOKENS__:'):
      continue

    if not is_json_obj(data):
      # Not a full JSON object on one line; count as fail.
      json_fail += 1
      continue

    try:
      obj = json.loads(data)
      json_ok += 1
    except Exception:
      json_fail += 1
      continue

    if isinstance(obj, dict):
      if isinstance(obj.get('usage'), dict):
        usage_chunks += 1
      choices = obj.get('choices')
      if isinstance(choices, list):
        choice_chunks += 1
        if len(choices) == 0 and isinstance(obj.get('usage'), dict):
          empty_choices_usage += 1

print(f"data: lines: {total} (DONE: {done})")
print(f"JSON OK: {json_ok}, JSON FAIL: {json_fail}")
print(f"Chunks with choices: {choice_chunks}")
print(f"Chunks with usage: {usage_chunks} (usage-only choices=[]: {empty_choices_usage})")
PY

echo ""
echo "--- Test complete ---"
