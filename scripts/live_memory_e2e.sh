#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Real-model, cross-session memory acceptance test. It intentionally uses the
# production CLI/Core/Host tool loop and an isolated database.

set -euo pipefail

if [[ "${NOLA_REAL_LLM_E2E:-}" != "1" ]]; then
    echo "Set NOLA_REAL_LLM_E2E=1 to run the real-model memory test." >&2
    exit 2
fi

for command in jq sqlite3 cargo; do
    if ! command -v "$command" >/dev/null 2>&1; then
        echo "Required command is unavailable: $command" >&2
        exit 2
    fi
done

run_with_timeout() {
    local timeout_seconds=$1
    shift
    local timeout_marker
    timeout_marker=$(mktemp "${TMPDIR:-/tmp}/nola-memory-timeout.XXXXXX")
    "$@" &
    local command_pid=$!
    (
        sleep "$timeout_seconds"
        if kill -0 "$command_pid" 2>/dev/null; then
            printf 'timed out\n' > "$timeout_marker"
            kill -TERM "$command_pid" 2>/dev/null || true
            sleep 5
            kill -KILL "$command_pid" 2>/dev/null || true
        fi
    ) &
    local watchdog_pid=$!
    local status
    if wait "$command_pid"; then
        status=0
    else
        status=$?
    fi
    kill "$watchdog_pid" 2>/dev/null || true
    wait "$watchdog_pid" 2>/dev/null || true
    if [[ -s "$timeout_marker" ]]; then
        status=124
    fi
    rm -f "$timeout_marker"
    return "$status"
}

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
source "$repo_root/scripts/e2e/cargo_paths.sh"
nola_bin=${NOLA_CLI_BIN:-"$(nola_cargo_binary "$repo_root" release nola)"}
source_config=${NOLA_CONFIG_PATH:-"$HOME/.nola/config.json"}
model_profile_db=${NOLA_MODEL_PROFILE_DB:-"$HOME/Library/Application Support/Nola/State/nola-v4.db"}

resolve_profile_field() {
    local field=$1
    [[ -f "$model_profile_db" ]] || return 0
    case "$field" in
        model)
            sqlite3 "$model_profile_db" \
                "SELECT model_name FROM model_profiles
                 WHERE selected=1 AND enabled=1 AND deleted_at_ms IS NULL AND runtime='remote'
                 ORDER BY updated_at_ms DESC LIMIT 1;" 2>/dev/null
            ;;
        endpoint)
            sqlite3 "$model_profile_db" \
                "SELECT COALESCE(json_extract(source_json,'$.baseURL'),
                                 json_extract(source_json,'$.base_url'),
                                 json_extract(source_json,'$.endpoint'),
                                 json_extract(source_json,'$.url'))
                 FROM model_profiles
                 WHERE selected=1 AND enabled=1 AND deleted_at_ms IS NULL AND runtime='remote'
                 ORDER BY updated_at_ms DESC LIMIT 1;" 2>/dev/null
            ;;
        api_key)
            sqlite3 "$model_profile_db" \
                "SELECT COALESCE(json_extract(source_json,'$.apiKey'),
                                 json_extract(source_json,'$.api_key'))
                 FROM model_profiles
                 WHERE selected=1 AND enabled=1 AND deleted_at_ms IS NULL AND runtime='remote'
                 ORDER BY updated_at_ms DESC LIMIT 1;" 2>/dev/null
            ;;
    esac
}

model_name=""
upstream_endpoint=""
model_api_key=""
model_source=""
explicit_model=${NOLA_REAL_E2E_MODEL:-}
explicit_endpoint=${NOLA_REAL_E2E_ENDPOINT:-}
explicit_api_key=${NOLA_REAL_E2E_API_KEY:-}
generic_model=${MODEL_NAME:-}
generic_endpoint=${MODEL_API_URL:-}
generic_api_key=${MODEL_API_KEY:-}
if [[ -n "$explicit_model" || -n "$explicit_endpoint" || -n "$explicit_api_key" ]]; then
    model_name=$explicit_model
    upstream_endpoint=$explicit_endpoint
    model_api_key=$explicit_api_key
    model_source=explicit-environment
elif [[ -n "$generic_model" && -n "$generic_endpoint" && -n "$generic_api_key" ]]; then
    model_name=$generic_model
    upstream_endpoint=$generic_endpoint
    model_api_key=$generic_api_key
    model_source=environment
else
    profile_model=$(resolve_profile_field model)
    profile_endpoint=$(resolve_profile_field endpoint)
    profile_api_key=$(resolve_profile_field api_key)
    if [[ -n "$profile_model" && -n "$profile_endpoint" && -n "$profile_api_key" ]]; then
        model_name=$profile_model
        upstream_endpoint=$profile_endpoint
        model_api_key=$profile_api_key
        model_source=nola-store
    elif [[ -f "$source_config" ]] && jq -e '.llm.endpoint and .llm.api_key and .llm.model' "$source_config" >/dev/null 2>&1; then
        model_name=$(jq -r '.llm.model' "$source_config")
        upstream_endpoint=$(jq -r '.llm.endpoint' "$source_config")
        model_api_key=$(jq -r '.llm.api_key' "$source_config")
        model_source=config
    fi
fi
if [[ -z "$model_name" || -z "$upstream_endpoint" || -z "$model_api_key" ]]; then
    echo "No complete atomic remote model configuration was found in explicit environment, generic environment, canonical Nola profile, or $source_config." >&2
    exit 2
fi
upstream_endpoint=${upstream_endpoint%/}
upstream_endpoint=${upstream_endpoint%/chat/completions}
printf '{"stage":"model-selection","model":"%s","endpoint":"%s","source":"%s"}\n' \
    "$model_name" "$upstream_endpoint" "$model_source"

if [[ "${NOLA_SKIP_CLI_BUILD:-0}" != "1" ]]; then
    run_with_timeout "${NOLA_REAL_E2E_BUILD_TIMEOUT_SECONDS:-600}" \
        cargo build --manifest-path "$repo_root/cli/Cargo.toml" --release --bin nola --quiet
fi
[[ -x "$nola_bin" ]] || { echo "Nola CLI binary is unavailable: $nola_bin" >&2; exit 2; }

evidence_root=$(mktemp -d "${TMPDIR:-/tmp}/nola-memory-real-e2e.XXXXXX")
test_home="$evidence_root/home"
project_root="$evidence_root/project"
test_config="$test_home/.nola/config.json"
mkdir -p "$test_home/.nola" "$project_root"
provider_trace="$evidence_root/provider.ndjson"
provider_ready="$evidence_root/provider-ready.json"
probe_pid=""

# Never leave a copied API key in the evidence directory, including when the
# provider probe or config write fails halfway through setup.
scrub_test_config() {
    printf '{}\n' > "$test_config"
    if [[ -n "$probe_pid" ]]; then
        kill -TERM "$probe_pid" 2>/dev/null || true
        wait "$probe_pid" 2>/dev/null || true
        probe_pid=""
    fi
}
trap scrub_test_config EXIT

python3 "$repo_root/scripts/e2e/provider_probe.py" \
    --upstream "$upstream_endpoint" \
    --log "$provider_trace" \
    --ready "$provider_ready" &
probe_pid=$!
for _ in {1..100}; do
    [[ -s "$provider_ready" ]] && break
    kill -0 "$probe_pid" 2>/dev/null || break
    sleep 0.05
done
[[ -s "$provider_ready" ]] || { echo "Provider evidence probe did not become ready." >&2; exit 2; }
proxy_endpoint=$(jq -er '.endpoint' "$provider_ready")
jq -n \
    --arg endpoint "$proxy_endpoint" \
    --arg api_key "$model_api_key" \
    --arg model "$model_name" \
    '{llm:{endpoint:$endpoint,api_key:$api_key,model:$model},network_policy:"allow"}' \
    > "$test_config"
chmod 600 "$test_config"

export HOME="$test_home"
export NOLA_DB_PATH="$evidence_root/nola.db"
export NOLA_CLI_INTERACTION_MODE=never

memory_code="memory-e2e-$(date +%s)-${RANDOM}"
writer="$evidence_root/write.ndjson"
reader="$evidence_root/recall.ndjson"
memory_mode=${NOLA_MEMORY_E2E_MODE:-explicit}
if [[ "$memory_mode" == "automatic" ]]; then
    export NOLA_LONG_TERM_MEMORY_ENABLED=1
    memory_code="Amber Lantern ${RANDOM}"
    writer_prompt="This is an explicit durable preference for future conversations: my preferred release-note closing phrase is '$memory_code'. Do not call memory_save or write files. Reply exactly MEMORY_WRITE_OK."
    reader_prompt="This is a new conversation. Reply with only my exact preferred release-note closing phrase from durable context. Do not infer or guess."
else
    writer_prompt="This is an explicit durable preference for future conversations: my release-note verification code is $memory_code. Use the internal long-term memory tool to save that exact preference. Do not use files. After the memory tool succeeds, reply exactly MEMORY_WRITE_OK."
    reader_prompt="This is a new conversation. Reply with only my exact release-note verification code from durable context. Do not infer or guess."
fi

run_with_timeout "${NOLA_REAL_E2E_TURN_TIMEOUT_SECONDS:-300}" "$nola_bin" \
    "$writer_prompt" \
    --project "$project_root" \
    --session memory-write-e2e \
    --json \
    > "$writer" \
    2> "$evidence_root/write.stderr"

if [[ "$memory_mode" == "automatic" ]]; then
    worker_inbox="$evidence_root/MemoryWorker/memory-write-e2e"
    worker_deadline=$((SECONDS + ${NOLA_REAL_E2E_WORKER_TIMEOUT_SECONDS:-180}))
    while ! find "$worker_inbox" -maxdepth 1 -type f -name 'memory-extract-*.json' -print -quit 2>/dev/null | grep -q .; do
        if (( SECONDS >= worker_deadline )); then
            echo "Automatic memory worker did not produce a bounded result." >&2
            exit 124
        fi
        sleep 0.1
    done
    run_with_timeout "${NOLA_REAL_E2E_TURN_TIMEOUT_SECONDS:-300}" \
        "$nola_bin" \
        "Import completed background work, then reply exactly MEMORY_WORKER_SETTLED." \
        --project "$project_root" \
        --session memory-write-e2e \
        --json \
        > "$evidence_root/settle.ndjson" \
        2> "$evidence_root/settle.stderr"
fi

run_with_timeout "${NOLA_REAL_E2E_TURN_TIMEOUT_SECONDS:-300}" "$nola_bin" \
    "$reader_prompt" \
    --project "$project_root" \
    --session memory-recall-e2e \
    --json \
    > "$reader" \
    2> "$evidence_root/recall.stderr"

write_summary=$(python3 "$repo_root/scripts/real_e2e_events.py" "$writer")
recall_summary=$(python3 "$repo_root/scripts/real_e2e_events.py" "$reader")
write_answer=$(jq -r '.assistant' <<<"$write_summary")
recall_answer=$(jq -r '.assistant' <<<"$recall_summary")
last_write_tool_sequence=$(jq -r '.lastToolSequence' <<<"$write_summary")
first_write_assistant_sequence=$(jq -r '.firstAssistantSequence // 0' <<<"$write_summary")
permission_count=$(jq -n --argjson write "$write_summary" --argjson recall "$recall_summary" \
    '$write.permissionCount + $recall.permissionCount')
stored_rows=$(sqlite3 "$NOLA_DB_PATH" \
    "select count(*) from memory_records where content like '%${memory_code}%';")
automatic_rows=0
if [[ "$memory_mode" == "automatic" ]]; then
    automatic_rows=$(sqlite3 "$NOLA_DB_PATH" \
        "select count(*) from memory_records
         where content like '%${memory_code}%'
           and json_extract(provenance_json,'$.kind')='agent_inference'
           and json_extract(provenance_json,'$.session_id')='memory-write-e2e'
           and json_array_length(json_extract(provenance_json,'$.item_ids'))>0;")
fi
printf '%s\n' "$write_answer" > "$evidence_root/write-answer.txt"
printf '%s\n' "$recall_answer" > "$evidence_root/recall-answer.txt"

fail() {
    echo "real_memory_e2e=failed: $1" >&2
    echo "evidence_root=$evidence_root" >&2
    scrub_test_config
    trap - EXIT
    exit 1
}

if [[ "$memory_mode" == "automatic" ]]; then
    jq -e '.toolsCompleted | index("memory_save") == null' <<<"$write_summary" >/dev/null \
        || fail "automatic writer unexpectedly called memory_save"
    jq -e '.toolsCompleted | length == 0' <<<"$recall_summary" >/dev/null \
        || fail "automatic recall did not use the selected resident memory"
    [[ "$automatic_rows" == "1" ]] \
        || fail "canonical memory row lacks automatic extraction provenance"
else
    jq -e '.toolsCompleted | index("memory_save") != null' <<<"$write_summary" >/dev/null \
        || fail "writer did not call memory_save"
    jq -e '.toolsCompleted | index("memory_search") != null' <<<"$recall_summary" >/dev/null \
        || fail "reader did not call memory_search"
fi
[[ "$write_answer" == *"MEMORY_WRITE_OK"* ]] \
    || fail "writer did not report successful completion"
[[ "$recall_answer" == "$memory_code" ]] \
    || fail "new conversation did not recall the exact stored value"
[[ "$first_write_assistant_sequence" == "0" || "$first_write_assistant_sequence" -gt "$last_write_tool_sequence" ]] \
    || fail "assistant text became visible before the last tool call"
[[ "$permission_count" == "0" ]] \
    || fail "internal memory tools requested user permission"
[[ "$stored_rows" == "1" ]] \
    || fail "memory was not persisted exactly once"

write_answer_exact=false
if [[ "$write_answer" == "MEMORY_WRITE_OK" ]]; then
    write_answer_exact=true
fi

python3 - "$repo_root" "$writer" "$reader" "$NOLA_DB_PATH" "$stored_rows" "$permission_count" <<'PY'
from pathlib import Path
import sys

root, writer, reader, database, stored_rows, permission_count = sys.argv[1:]
sys.path.insert(0, str(Path(root) / "scripts"))
from real_e2e_evidence import record_step

record_step(
    "cross-session-memory-save-recall",
    platforms=["cli"],
    modules=["agent", "core", "provider", "memory", "persistence"],
    boundaries=["cli", "agent-host", "provider", "persistence"],
    assertions={
        "memory_saved": int(stored_rows) == 1,
        "memory_recalled_in_new_session": True,
        "internal_memory_needs_no_permission": int(permission_count) == 0,
        "assistant_waited_for_tool": True,
    },
    metrics={"storedRows": int(stored_rows), "permissionEvents": int(permission_count)},
    assertion_evidence={
        "memory_saved": ["runtime-db"],
        "memory_recalled_in_new_session": ["recall-events", "runtime-db"],
        "internal_memory_needs_no_permission": ["write-events", "recall-events"],
        "assistant_waited_for_tool": ["write-events"],
    },
    observations=[
        {"id": "provider-traffic", "kind": "provider_trace", "path": Path(writer).with_name("provider.ndjson")},
        {"id": "write-events", "kind": "event_stream", "path": writer},
        {"id": "recall-events", "kind": "event_stream", "path": reader},
        {"id": "runtime-db", "kind": "persistence_store", "path": database},
    ],
    artifacts=[writer, reader, database],
)
PY

scrub_test_config
trap - EXIT

cat <<EOF
real_memory_e2e=passed
write_answer_contains_marker=true
write_answer_exact=$write_answer_exact
recall_answer=$recall_answer
permission_events=$permission_count
stored_rows=$stored_rows
premature_visible_text=0
evidence_root=$evidence_root
EOF
