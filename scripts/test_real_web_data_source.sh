#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SOURCE_CONFIG="${NOLA_REAL_LLM_CONFIG:-$HOME/.nola/config.json}"
BINARY="${NOLA_REAL_WEB_BINARY:-$ROOT/target/release/nola}"

if [[ ! -x "$BINARY" ]]; then
  printf 'missing Web host binary: %s\n' "$BINARY" >&2
  printf 'build it with: cd %s && cargo build --release --bin nola\n' "$ROOT" >&2
  exit 1
fi

if [[ ! -f "$SOURCE_CONFIG" ]]; then
  printf 'missing real LLM config: %s\n' "$SOURCE_CONFIG" >&2
  exit 1
fi

endpoint="$(jq -er '.llm.endpoint // .endpoint' "$SOURCE_CONFIG")"
model="$(jq -er '.llm.model // .model' "$SOURCE_CONFIG")"
api_key="$(jq -r '.llm.api_key // .api_key // ""' "$SOURCE_CONFIG")"

case "$endpoint" in
  http://127.0.0.1:*|https://127.0.0.1:*|http://localhost:*|https://localhost:*)
    ;;
  http://*|https://*)
    ;;
  *)
    printf 'real LLM endpoint must be HTTP(S)\n' >&2
    exit 1
    ;;
esac

tmp="$(mktemp -d "${TMPDIR:-/tmp}/nola-real-web-source.XXXXXX")"
server_pid=""
success=0

cleanup() {
  if [[ -n "$server_pid" ]]; then
    kill "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
  fi
  if [[ "$success" == 1 ]]; then
    rm -rf "$tmp"
  else
    printf 'real Web data-source diagnostics preserved at %s\n' "$tmp" >&2
  fi
}
trap cleanup EXIT

isolated_home="$tmp/home"
mkdir -p "$isolated_home/.nola"
jq -n \
  --arg endpoint "$endpoint" \
  --arg api_key "$api_key" \
  --arg model "$model" \
  '{llm:{endpoint:$endpoint,api_key:$api_key,model:$model}}' \
  > "$isolated_home/.nola/config.json"
chmod 600 "$isolated_home/.nola/config.json"

marker="WEB-SOURCE-$(uuidgen | tr '[:lower:]' '[:upper:]')"
source_file="$tmp/verified-source.txt"
printf 'headline=%s\nclassification=verified-test-only\n' "$marker" > "$source_file"

port="$(python3 - <<'PY'
import socket

with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
    sock.bind(("127.0.0.1", 0))
    print(sock.getsockname()[1])
PY
)"
base_url="http://127.0.0.1:$port"

HOME="$isolated_home" \
NOLA_DB_PATH="$tmp/nola.db" \
NOLA_CLI_DATA_SOURCES_STATE="$tmp/data-sources.json" \
NOLA_WEB_ROOT="$ROOT/apps/web" \
"$BINARY" web --host 127.0.0.1 --port "$port" \
  >"$tmp/web.stdout.log" 2>"$tmp/web.stderr.log" &
server_pid=$!

ready=0
for _ in $(seq 1 100); do
  if curl --silent --fail --max-time 1 "$base_url/api/health" >"$tmp/health.json" 2>/dev/null; then
    ready=1
    break
  fi
  if ! kill -0 "$server_pid" 2>/dev/null; then
    break
  fi
  sleep 0.1
done
if [[ "$ready" != 1 ]]; then
  printf 'Web host did not become ready\n' >&2
  exit 1
fi

jq -n \
  --arg name "Real Web grounded source" \
  --arg location "$source_file" \
  '{name:$name,kind:"localFile",location:$location,accessPolicy:"defaultEnabled",maximumAgeMinutes:15}' \
  > "$tmp/create-source.request.json"

create_status="$(curl --silent --show-error --max-time 20 \
  -o "$tmp/create-source.response.json" -w '%{http_code}' \
  -H 'Content-Type: application/json' \
  --data-binary "@$tmp/create-source.request.json" \
  "$base_url/api/data-sources")"
if [[ "$create_status" != 201 ]]; then
  printf 'data-source creation failed with HTTP %s\n' "$create_status" >&2
  exit 1
fi

source_id="$(jq -er '.source.id // .dataSource.id // .id' "$tmp/create-source.response.json")"
jq -e '
  (.source // .dataSource // .) as $source
  | ((.tested // $source.tested // false) == true)
  and (($source.validationContract.validationState // $source.validationContract.status // $source.validationContract.state // "")
       | IN("verified", "valid", "ready"))
' "$tmp/create-source.response.json" >/dev/null || {
  printf 'created data source is not verified and ready\n' >&2
  exit 1
}

test_status="$(curl --silent --show-error --max-time 20 \
  -o "$tmp/test-source.response.json" -w '%{http_code}' \
  -X POST "$base_url/api/data-sources/$source_id/test")"
if [[ "$test_status" != 200 ]]; then
  printf 'data-source validation failed with HTTP %s\n' "$test_status" >&2
  exit 1
fi

conversation_status="$(curl --silent --show-error --max-time 10 \
  -o "$tmp/create-conversation.response.json" -w '%{http_code}' \
  -H 'Content-Type: application/json' \
  --data '{}' \
  "$base_url/api/chat/conversations")"
if [[ "$conversation_status" != 200 && "$conversation_status" != 201 ]]; then
  printf 'conversation creation failed with HTTP %s\n' "$conversation_status" >&2
  exit 1
fi
conversation_id="$(jq -er '.conversationId // .conversation.id // .id' "$tmp/create-conversation.response.json")"
submission_id="$(uuidgen | tr '[:upper:]' '[:lower:]')"

jq -n \
  --arg conversation_id "$conversation_id" \
  --arg submission_id "$submission_id" \
  '{
    conversationId:$conversation_id,
    submission_id:$submission_id,
    user_message_id:($submission_id + ":user"),
    assistant_message_id:($submission_id + ":assistant"),
    execution_backend:"nola",
    agentMode:false,
    max_iterations:4,
    messages:[{
      role:"user",
      content:"请根据这个会话已启用且验证成功的数据源，用一句自然中文告诉我 headline 是什么，并说明它对应的 classification。不要猜测，也不要调用与回答无关的工具。"
    }]
  }' > "$tmp/chat.request.json"

chat_status="$(curl --silent --show-error --max-time 180 \
  -o "$tmp/chat.response.sse" -w '%{http_code}' \
  -H 'Content-Type: application/json' \
  -H 'Accept: text/event-stream' \
  --data-binary "@$tmp/chat.request.json" \
  "$base_url/api/chat/agent/v2/stream")"
if [[ "$chat_status" != 200 ]]; then
  printf 'real grounded chat failed with HTTP %s\n' "$chat_status" >&2
  exit 1
fi

answer="$(sed -n 's/^data: \({.*\)$/\1/p' "$tmp/chat.response.sse" \
  | jq -rs '[.[] | select(.type == "core_part_updated") | .part | select((.kind // .type) == "text") | {id, text}] | group_by(.id) | map(last.text) | join("")')"
ANSWER="$answer" MARKER="$marker" python3 - <<'PY'
import os
import sys

answer = os.environ["ANSWER"]
marker = os.environ["MARKER"]
if answer.count(marker) != 1:
    print("real model did not ground the answer in the unique headline", file=sys.stderr)
    sys.exit(1)
if "verified-test-only" not in answer:
    print("real model omitted the source-grounded classification", file=sys.stderr)
    sys.exit(1)
if len(answer.strip()) < len(marker) + len("verified-test-only") + 4:
    print("real model did not produce a natural grounded response", file=sys.stderr)
    sys.exit(1)
PY

curl --silent --show-error --fail --max-time 10 \
  "$base_url/api/data-sources/usage" > "$tmp/usage.response.json"
jq -e \
  --arg source_id "$source_id" \
  --arg conversation_id "$conversation_id" '
    [.. | objects
      | select((.sourceId? == $source_id or .source_id? == $source_id)
        and (.conversationId? == $conversation_id or .conversation_id? == $conversation_id)
        and (.consumer? == "web_chat")
        and (.purpose? == "conversation_context"))]
    | length == 1
  ' "$tmp/usage.response.json" >/dev/null || {
  printf 'missing exact Web conversation data-source usage record\n' >&2
  exit 1
}

if [[ -f "$tmp/data-sources.json" ]] \
  && rg --fixed-strings --quiet "$marker" "$tmp/data-sources.json"; then
  printf 'raw data-source content leaked into persisted source configuration\n' >&2
  exit 1
fi

printf 'real Web data-source grounding passed: source=%s conversation=%s model=%s\n' \
  "$source_id" "$conversation_id" "$model"
success=1
