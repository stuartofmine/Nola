#!/usr/bin/env sh
# scripts/test_claude_code_auth.sh
# Verify whether a custom Anthropic-compatible base URL and auth token work
# both at the raw HTTP layer and with the local Claude Code CLI.

set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$SCRIPT_DIR"

SETTINGS_FILE="${CLAUDE_SETTINGS_FILE:-$HOME/.claude/settings.json}"
API_ONLY=0
CLI_ONLY=0
BASE_URL="${ANTHROPIC_BASE_URL:-}"
AUTH_TOKEN="${ANTHROPIC_AUTH_TOKEN:-}"
MODEL="${ANTHROPIC_MODEL:-}"

usage() {
    cat <<'EOF'
Usage:
  ./scripts/test_claude_code_auth.sh [options]

Options:
  --base-url URL         Override ANTHROPIC_BASE_URL
  --auth-token TOKEN     Override ANTHROPIC_AUTH_TOKEN
  --model MODEL          Override ANTHROPIC_MODEL
  --api-only             Only test raw HTTP API compatibility
  --cli-only             Only test local Claude Code CLI
  -h, --help             Show this help

Resolution order:
  1. Command-line flags
  2. Current shell environment
  3. ~/.claude/settings.json

Examples:
  ./scripts/test_claude_code_auth.sh
  ./scripts/test_claude_code_auth.sh --api-only
  ./scripts/test_claude_code_auth.sh --base-url https://example.com/api/anthropic --auth-token '***'
EOF
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --base-url)
            BASE_URL="$2"
            shift 2
            ;;
        --auth-token)
            AUTH_TOKEN="$2"
            shift 2
            ;;
        --model)
            MODEL="$2"
            shift 2
            ;;
        --api-only)
            API_ONLY=1
            shift
            ;;
        --cli-only)
            CLI_ONLY=1
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "Unknown option: $1" >&2
            usage >&2
            exit 2
            ;;
    esac
done

if [ "$API_ONLY" -eq 1 ] && [ "$CLI_ONLY" -eq 1 ]; then
    echo "--api-only and --cli-only cannot be used together" >&2
    exit 2
fi

have_cmd() {
    command -v "$1" >/dev/null 2>&1
}

read_setting() {
    key="$1"
    if [ ! -f "$SETTINGS_FILE" ]; then
        return 1
    fi

    if have_cmd jq; then
        jq -r --arg key "$key" '.env[$key] // empty' "$SETTINGS_FILE"
        return 0
    fi

    python3 - "$SETTINGS_FILE" "$key" <<'PY'
import json
import sys

path, key = sys.argv[1], sys.argv[2]
with open(path, 'r', encoding='utf-8') as handle:
    data = json.load(handle)
print(data.get('env', {}).get(key, ''))
PY
}

if [ -z "$BASE_URL" ]; then
    BASE_URL="$(read_setting ANTHROPIC_BASE_URL 2>/dev/null || true)"
fi

if [ -z "$AUTH_TOKEN" ]; then
    AUTH_TOKEN="$(read_setting ANTHROPIC_AUTH_TOKEN 2>/dev/null || true)"
fi

if [ -z "$MODEL" ]; then
    MODEL="$(read_setting ANTHROPIC_MODEL 2>/dev/null || true)"
fi

if [ -z "$BASE_URL" ]; then
    echo "Missing ANTHROPIC_BASE_URL. Set it in env, pass --base-url, or populate $SETTINGS_FILE." >&2
    exit 2
fi

if [ -z "$AUTH_TOKEN" ]; then
    echo "Missing ANTHROPIC_AUTH_TOKEN. Set it in env, pass --auth-token, or populate $SETTINGS_FILE." >&2
    exit 2
fi

if [ -z "$MODEL" ]; then
    MODEL="claude-3-5-sonnet-latest"
fi

mask_token() {
    value="$1"
    length=${#value}
    if [ "$length" -le 8 ]; then
        printf '********\n'
        return
    fi
    prefix=$(printf '%s' "$value" | cut -c 1-4)
    suffix=$(printf '%s' "$value" | awk '{print substr($0, length($0)-3)}')
    printf '%s****%s\n' "$prefix" "$suffix"
}

normalize_base() {
    case "$1" in
        */)
            printf '%s\n' "${1%/}"
            ;;
        *)
            printf '%s\n' "$1"
            ;;
    esac
}

join_endpoint() {
    base="$1"
    suffix="$2"
    case "$base" in
        */v1)
            printf '%s%s\n' "$base" "${suffix#/v1}"
            ;;
        */v1/*)
            printf '%s\n' "$base"
            ;;
        *)
            printf '%s%s\n' "$base" "$suffix"
            ;;
    esac
}

BASE_URL=$(normalize_base "$BASE_URL")
MASKED_TOKEN=$(mask_token "$AUTH_TOKEN")

echo "== Claude Code connectivity check =="
echo "settings file: $SETTINGS_FILE"
echo "base url:      $BASE_URL"
echo "model:         $MODEL"
echo "auth token:    $MASKED_TOKEN"
echo

API_STATUS=0
CLI_STATUS=0

probe_api() {
    url="$1"
    auth_mode="$2"
    body_file=$(mktemp)
    headers_file=$(mktemp)
    response_file=$(mktemp)
    error_file=$(mktemp)

    cat >"$body_file" <<EOF
{"model":"$MODEL","max_tokens":8,"messages":[{"role":"user","content":"Reply with OK"}]}
EOF

    if [ "$auth_mode" = "x-api-key" ]; then
        http_code=$(curl -sS -o "$response_file" -D "$headers_file" -w '%{http_code}' \
            -X POST "$url" \
            --connect-timeout 8 \
            --max-time 20 \
            -H "content-type: application/json" \
            -H "anthropic-version: 2023-06-01" \
            -H "x-api-key: $AUTH_TOKEN" \
            --data-binary @"$body_file" 2>"$error_file" || true)
    else
        http_code=$(curl -sS -o "$response_file" -D "$headers_file" -w '%{http_code}' \
            -X POST "$url" \
            --connect-timeout 8 \
            --max-time 20 \
            -H "content-type: application/json" \
            -H "anthropic-version: 2023-06-01" \
            -H "authorization: Bearer $AUTH_TOKEN" \
            --data-binary @"$body_file" 2>"$error_file" || true)
    fi

    echo "--- response preview ($auth_mode) ---" >&2
    if [ -s "$response_file" ]; then
        head -c 400 "$response_file" >&2 || true
        echo >&2
    elif [ -s "$error_file" ]; then
        head -c 400 "$error_file" >&2 || true
        echo >&2
    else
        echo "(no response body)" >&2
    fi

    rm -f "$body_file" "$headers_file" "$response_file" "$error_file"
    printf '%s\n' "$http_code"
}

run_api_check() {
    candidate_1=$(join_endpoint "$BASE_URL" "/v1/messages")
    candidate_2=$(join_endpoint "$BASE_URL" "/messages")

    echo "[1/2] Raw API probe"
    echo "Trying endpoint: $candidate_1"
    code=$(probe_api "$candidate_1" "x-api-key")
    api_code=$(printf '%s\n' "$code" | tail -n 1)
    if [ "$api_code" = "200" ]; then
        echo "API probe succeeded with x-api-key on $candidate_1"
        return 0
    fi

    echo
    echo "Trying endpoint: $candidate_1"
    code=$(probe_api "$candidate_1" "bearer")
    api_code=$(printf '%s\n' "$code" | tail -n 1)
    if [ "$api_code" = "200" ]; then
        echo "API probe succeeded with Bearer token on $candidate_1"
        return 0
    fi

    if [ "$candidate_2" != "$candidate_1" ]; then
        echo
        echo "Trying endpoint: $candidate_2"
        code=$(probe_api "$candidate_2" "x-api-key")
        api_code=$(printf '%s\n' "$code" | tail -n 1)
        if [ "$api_code" = "200" ]; then
            echo "API probe succeeded with x-api-key on $candidate_2"
            return 0
        fi

        echo
        echo "Trying endpoint: $candidate_2"
        code=$(probe_api "$candidate_2" "bearer")
        api_code=$(printf '%s\n' "$code" | tail -n 1)
        if [ "$api_code" = "200" ]; then
            echo "API probe succeeded with Bearer token on $candidate_2"
            return 0
        fi
    fi

    echo "API probe failed. Check URL path, token format, supported auth header, and model name." >&2
    return 1
}

run_cli_check() {
    echo
    echo "[2/2] Claude Code CLI probe"
    if ! have_cmd claude; then
        echo "Claude CLI not found in PATH." >&2
        return 1
    fi

    debug_log=$(mktemp)
    prompt_file=$(mktemp)
    printf '%s\n' 'Reply with exactly: OK' >"$prompt_file"

    set +e
    cli_output=$(ANTHROPIC_BASE_URL="$BASE_URL" \
        ANTHROPIC_AUTH_TOKEN="$AUTH_TOKEN" \
        ANTHROPIC_MODEL="$MODEL" \
        CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS=1 \
        claude -p --dangerously-skip-permissions --debug-file "$debug_log" <"$prompt_file" 2>&1)
    cli_exit=$?
    set -e

    rm -f "$prompt_file"

    echo "--- claude output preview ---"
    printf '%s\n' "$cli_output" | head -n 20

    if [ "$cli_exit" -eq 0 ]; then
        echo "Claude CLI probe succeeded. Debug log: $debug_log"
        return 0
    fi

    echo "Claude CLI probe failed with exit code $cli_exit. Debug log: $debug_log" >&2
    return 1
}

if [ "$CLI_ONLY" -ne 1 ]; then
    if ! run_api_check; then
        API_STATUS=1
    fi
fi

if [ "$API_ONLY" -ne 1 ]; then
    if ! run_cli_check; then
        CLI_STATUS=1
    fi
fi

echo
echo "== Summary =="
if [ "$API_ONLY" -ne 1 ]; then
    echo "raw api:   $([ "$API_STATUS" -eq 0 ] && echo PASS || echo FAIL)"
fi
if [ "$CLI_ONLY" -ne 1 ]; then
    echo "claude:    $([ "$CLI_STATUS" -eq 0 ] && echo PASS || echo FAIL)"
fi

if [ "$API_STATUS" -ne 0 ] || [ "$CLI_STATUS" -ne 0 ]; then
    exit 1
fi