#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# Real, opt-in product E2E for installed external execution units.

set -eu

if [ "${NOLA_RUN_REAL_EXTERNAL_EXECUTOR_E2E:-0}" != "1" ]; then
    echo "skip: set NOLA_RUN_REAL_EXTERNAL_EXECUTOR_E2E=1"
    exit 0
fi

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
. "$repo_root/scripts/e2e/cargo_paths.sh"
host=${NOLA_AGENT_HOST_BIN:-"$(nola_cargo_binary "$repo_root" debug nola-agent-host)"}
if [ ! -x "$host" ]; then
    echo "error: build nola-agent-host first: $host" >&2
    exit 1
fi

catalog=$($host --list-execution-backends)

run_turn() {
    backend=$1
    conversation=$2
    workspace=$3
    prompt=$4
    upstream_session=${5:-}
    model=${6:-}
    output=$7
    request=$(jq -nc \
        --arg prompt "$prompt" \
        --arg backend "$backend" \
        --arg model "$model" \
        --arg upstream_session "$upstream_session" \
        '{prompt:$prompt,context_components:[],execution_backend:$backend}
         + if $model == "" then {} else {model:$model} end
         + if $upstream_session == "" then {} else {external_session_id:$upstream_session} end')
    printf 'run: %s %s\n' "$backend" "${upstream_session:+resume}" >&2
    printf '%s\n' "$request" | \
        NOLA_DESKTOP_HOST=1 \
        NOLA_CLIENT_SURFACE=mac \
        NOLA_DB_PATH="$workspace/nola.db" \
        /usr/bin/perl -e 'alarm shift; exec @ARGV' 180 \
        "$host" run --json --host-request-stdin \
            --session "$conversation" --project "$workspace" > "$output"
}

verify_backend() {
    backend=$1
    marker=$2
    workspace=$(mktemp -d "/tmp/nola-${backend}-real-e2e.XXXXXX")
    trap 'rm -rf "$workspace"' EXIT HUP INT TERM
    suffix=$(basename "$workspace" | tr -cd '[:alnum:]_-')
    conversation="external-${backend}-${suffix}"
    case "$backend" in
        codex) model=${NOLA_CODEX_MODEL:-} ;;
        claude_code) model=${NOLA_CLAUDE_CODE_MODEL:-} ;;
        opencode) model=${NOLA_OPENCODE_MODEL:-} ;;
        *) model= ;;
    esac

    run_turn "$backend" "$conversation" "$workspace" \
        "For this conversation, the fictional plant is named $marker. Reply with exactly STORED." \
        "" "$model" "$workspace/first.jsonl"
    upstream_session=$(jq -r \
        'select(.type == "external_session_started") | .external_session_id' \
        "$workspace/first.jsonl" | head -n 1)
    [ -n "$upstream_session" ]
    [ "$(jq -s '[.[] | select(.type == "done")] | length' "$workspace/first.jsonl")" = "1" ]

    run_turn "$backend" "$conversation" "$workspace" \
        "What was the fictional plant named? Reply with only its name." \
        "$upstream_session" "$model" "$workspace/second.jsonl"
    answer=$(jq -rs \
        '[.[] | select(.type == "assistant_delta" and .phase == "final_answer") | .delta] | join("")' \
        "$workspace/second.jsonl")
    resumed_session=$(jq -r \
        'select(.type == "external_session_started") | .external_session_id' \
        "$workspace/second.jsonl" | head -n 1)
    [ "$answer" = "$marker" ]
    [ "$resumed_session" = "$upstream_session" ]
    [ "$(jq -s '[.[] | select(.type == "done")] | length' "$workspace/second.jsonl")" = "1" ]
    printf 'pass: %s session=%s\n' "$backend" "$upstream_session"
    rm -rf "$workspace"
    trap - EXIT HUP INT TERM
}

available=$(printf '%s' "$catalog" | jq -r \
    '.[] | select(.id != "nola" and .availability == "available") | .id')
if [ -n "${NOLA_EXTERNAL_EXECUTOR_BACKENDS:-}" ]; then
    requested=$(printf '%s' "$NOLA_EXTERNAL_EXECUTOR_BACKENDS" | tr ',' ' ')
    filtered=
    for backend in $available; do
        for candidate in $requested; do
            if [ "$backend" = "$candidate" ]; then
                filtered="$filtered $backend"
            fi
        done
    done
    available=$(printf '%s' "$filtered" | xargs)
fi
if [ -z "$available" ]; then
    echo "error: no installed and authenticated external execution unit" >&2
    exit 1
fi

for backend in $available; do
    case "$backend" in
        codex) marker="CobaltFern7319" ;;
        claude_code) marker="SilverFern4826" ;;
        opencode) marker="AmberFern5931" ;;
        *) echo "error: unexpected available backend: $backend" >&2; exit 1 ;;
    esac
    verify_backend "$backend" "$marker"
done

printf '%s\n' "$catalog" | jq -e \
    '.[] | select(.id == "opencode") | .availability != "available" or .executable_path != null' \
    >/dev/null
