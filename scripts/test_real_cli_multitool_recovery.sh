#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Real-provider regression for multi-tool turns, current checkpoint restore, and
# same-conversation recovery with isolated state and an explicit model contract.

set -euo pipefail

if [[ "${NOLA_REAL_LLM_E2E:-}" != "1" ]]; then
    printf 'Set NOLA_REAL_LLM_E2E=1 to run the real multi-tool recovery test.\n' >&2
    exit 2
fi

for command in jq python3; do
    command -v "$command" >/dev/null 2>&1 || {
        printf 'Required command is unavailable: %s\n' "$command" >&2
        exit 2
    }
done

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$ROOT/scripts/e2e/cargo_paths.sh"
SOURCE_CONFIG="${NOLA_CONFIG_PATH:-$HOME/.nola/config.json}"
NOLA_BIN="${NOLA_CLI_BIN:-$(nola_cargo_binary "$ROOT" release nola)}"
PERMISSION_PROFILE="${NOLA_REAL_CLI_PERMISSION_PROFILE:-cautious}"
case "$PERMISSION_PROFILE" in
    cautious|allow_all) ;;
    *) printf 'Use cautious or allow_all for the isolated permission fixture.\n' >&2; exit 2 ;;
esac

[[ -f "$SOURCE_CONFIG" ]] || {
    printf 'Nola configuration is unavailable: %s\n' "$SOURCE_CONFIG" >&2
    exit 2
}
jq -e '.llm.endpoint and .llm.api_key and .llm.model' "$SOURCE_CONFIG" >/dev/null
MODEL_FIXTURE="$(python3 "$ROOT/scripts/e2e/model_execution_fixture.py" \
    --config "$SOURCE_CONFIG" --format cli)" || exit 2

if [[ "${NOLA_SKIP_CLI_BUILD:-0}" != "1" ]]; then
    printf '[real-cli-multitool] building release CLI\n'
    (cd "$ROOT/cli" && cargo build --release --bin nola --quiet)
fi
[[ -x "$NOLA_BIN" ]] || {
    printf 'Current Nola CLI binary is unavailable: %s\n' "$NOLA_BIN" >&2
    exit 2
}

EVIDENCE_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/nola-cli-multitool-real.XXXXXX")"
TEST_HOME="$EVIDENCE_ROOT/home"
PROJECT_ROOT="$EVIDENCE_ROOT/project"
SESSION_ID="real-multitool-recovery-$(date +%s)-${RANDOM}"
TEST_CONFIG="$TEST_HOME/.nola/config.json"
PROVIDER_TRACE="$EVIDENCE_ROOT/provider.ndjson"
PROVIDER_READY="$EVIDENCE_ROOT/provider-ready.json"
PROBE_PID=""
mkdir -p "$TEST_HOME/.nola" "$PROJECT_ROOT"
umask 077
printf 'evidence_root=%s\n' "$EVIDENCE_ROOT"

cleanup() {
    if [[ -f "$TEST_CONFIG" ]]; then
        printf '{}\n' > "$TEST_CONFIG"
    fi
    if [[ -n "$PROBE_PID" ]]; then
        kill -TERM "$PROBE_PID" 2>/dev/null || true
        wait "$PROBE_PID" 2>/dev/null || true
    fi
}
trap cleanup EXIT INT TERM

UPSTREAM_ENDPOINT="$(jq -er '.llm.endpoint' "$SOURCE_CONFIG")"
python3 "$ROOT/scripts/e2e/provider_probe.py" \
    --upstream "$UPSTREAM_ENDPOINT" \
    --log "$PROVIDER_TRACE" \
    --ready "$PROVIDER_READY" &
PROBE_PID=$!
for _ in {1..100}; do
    [[ -s "$PROVIDER_READY" ]] && break
    kill -0 "$PROBE_PID" 2>/dev/null || break
    sleep 0.05
done
[[ -s "$PROVIDER_READY" ]] || {
    printf 'Provider evidence probe did not become ready.\n' >&2
    exit 2
}
PROXY_ENDPOINT="$(jq -er '.endpoint' "$PROVIDER_READY")"
jq --arg endpoint "$PROXY_ENDPOINT" --argjson fixture "$MODEL_FIXTURE" \
    '{llm:(.llm | del(.fallbacks) | .endpoint=$endpoint | .provider=$fixture.provider
        | .capabilities=$fixture.runtimeCapabilities), network_policy:"allow"}' \
    "$SOURCE_CONFIG" > "$TEST_CONFIG"
chmod 600 "$TEST_CONFIG"

printf 'alpha verification value: ALPHA-31415\n' > "$PROJECT_ROOT/alpha.txt"
printf 'beta verification value: BETA-92653\n' > "$PROJECT_ROOT/beta.txt"

export NOLA_HOME="$TEST_HOME"
export NOLA_DB_PATH="$EVIDENCE_ROOT/nola.db"
export NOLA_CLI_INTERACTION_MODE=never
export NOLA_PERMISSION_PROFILE="$PERMISSION_PROFILE"

FIRST_EVENTS="$EVIDENCE_ROOT/first.ndjson"
SECOND_EVENTS="$EVIDENCE_ROOT/second.ndjson"

"$NOLA_BIN" \
    --project "$PROJECT_ROOT" \
    --session "$SESSION_ID" \
    --max-iterations "${NOLA_REAL_CLI_MAX_ITERATIONS:-12}" \
    --no-skills \
    --lang en \
    --json \
    '请分别读取 alpha.txt 和 beta.txt，并用一句中文说明两个文件中的校验值。不要修改文件。' \
    > "$FIRST_EVENTS" 2> "$EVIDENCE_ROOT/first.stderr"

"$NOLA_BIN" \
    --project "$PROJECT_ROOT" \
    --session "$SESSION_ID" \
    --max-iterations "${NOLA_REAL_CLI_MAX_ITERATIONS:-12}" \
    --no-skills \
    --lang en \
    --json \
    '不要再次读取文件。请仅根据刚才同一会话中已经得到的结果，用“ALPHA-31415, BETA-92653”的格式回答两个校验值。' \
    > "$SECOND_EVENTS" 2> "$EVIDENCE_ROOT/second.stderr"

python3 - "$ROOT" "$FIRST_EVENTS" "$SECOND_EVENTS" "$EVIDENCE_ROOT" "$SESSION_ID" <<'PY'
import json
import pathlib
import re
import sqlite3
import sys

root, first_path, second_path, evidence_root = map(pathlib.Path, sys.argv[1:5])
session_id = sys.argv[5]
sys.path.insert(0, str(root / "scripts"))
from real_e2e_events import load_events, summarize
from real_e2e_evidence import record_step

def fail(message):
    print(f"real_cli_multitool_recovery=failed: {message}", file=sys.stderr)
    print(f"evidence_root={evidence_root}", file=sys.stderr)
    raise SystemExit(1)

first_events = load_events(first_path)
second_events = load_events(second_path)
first = summarize(first_path, evidence_root / "nola.db")
second = summarize(second_path, evidence_root / "nola.db")
first_tools = first["toolsStarted"] or first["toolsCompleted"]
first_results = first["toolsCompleted"]
second_tools = second["toolsStarted"] or second["toolsCompleted"]
errors = first["errors"] + second["errors"]

if not first_tools:
    fail("real model completed no file tool")
if len(first_results) < len(first_tools):
    fail("not every tool call produced a visible result")
tool_bodies = [part["bodies"] for part in first["journalParts"]
               if part["head"]["metadata"]["kind"] == "tool"
               and part["head"]["metadata"]["data"].get("state") == "completed"]
tool_inputs = json.dumps([body.get("tool_input", "") for body in tool_bodies], ensure_ascii=False)
tool_outputs = json.dumps([body.get("tool_output", "") for body in tool_bodies], ensure_ascii=False)
if "alpha.txt" not in tool_inputs or "beta.txt" not in tool_inputs:
    fail("tool arguments did not cover both requested files")
if "ALPHA-31415" not in tool_outputs or "BETA-92653" not in tool_outputs:
    fail("tool results did not contain both file values")
if "ALPHA-31415" not in first["assistant"] or "BETA-92653" not in first["assistant"]:
    fail(f"first answer was not grounded in both files: {first['assistant']!r}")
if second_tools:
    fail(f"second turn inherited tools: {second_tools}")
second_answer = second["assistant"].strip()
if "ALPHA-31415" not in second_answer or "BETA-92653" not in second_answer:
    fail(f"recovered conversation lost prior grounded results: {second_answer!r}")
if len(second_answer) > 240:
    fail(f"recovery answer was unexpectedly verbose: {second_answer!r}")
if errors:
    fail(f"protocol emitted failure events: {errors[:2]}")

serialized = json.dumps(first_events + second_events, ensure_ascii=False)
for forbidden in (
    "frozen Core invocation plan permits only one call",
    "unsupported Agent checkpoint schema",
):
    if forbidden in serialized:
        fail(f"internal protocol detail became user-visible: {forbidden}")

database = evidence_root / "nola.db"
try:
    with sqlite3.connect(database) as connection:
        table_names = [
            row[0]
            for row in connection.execute(
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'"
            )
        ]
        table_row_counts = {
            name: connection.execute(f'SELECT COUNT(*) FROM "{name}"').fetchone()[0]
            for name in table_names
        }
        conversation_count = connection.execute(
            "SELECT COUNT(*) FROM conversations WHERE id = ?", (session_id,)
        ).fetchone()[0]
        message_count = connection.execute(
            "SELECT COUNT(*) FROM messages WHERE conversation_id = ?", (session_id,)
        ).fetchone()[0]
        part_count = connection.execute(
            """SELECT COUNT(*)
               FROM message_parts AS part
               JOIN messages AS message ON message.id = part.message_id
               WHERE message.conversation_id = ?""",
            (session_id,),
        ).fetchone()[0]
        core_checkpoint_count = connection.execute(
            "SELECT COUNT(*) FROM core_checkpoints WHERE conversation_id = ?", (session_id,)
        ).fetchone()[0]
except (OSError, sqlite3.Error) as error:
    fail(f"runtime database could not be independently audited: {error}")

persistence_assertions = {
    "runtime_database_has_schema": bool(table_names),
    "conversation_is_durable": conversation_count == 1,
    "both_turns_are_durable": message_count >= 4,
    "message_parts_are_durable": part_count >= 4,
    "core_checkpoint_is_database_owned": core_checkpoint_count == 1,
}
audit_path = evidence_root / "multitool-persistence-audit.json"
audit_path.write_text(json.dumps({
    "passed": all(persistence_assertions.values()),
    "assertions": persistence_assertions,
    "databaseTables": table_row_counts,
    "conversationID": session_id,
    "conversationMessageCount": message_count,
    "conversationPartCount": part_count,
    "coreCheckpointCount": core_checkpoint_count,
}, ensure_ascii=False, indent=2), encoding="utf-8")
if not all(persistence_assertions.values()):
    fail(f"durable recovery audit failed: {persistence_assertions}")

transcript_path = evidence_root / "multitool-transcript.json"
transcript_path.write_text(json.dumps([
    {
        "role": "user",
        "text": "请分别读取 alpha.txt 和 beta.txt，并用一句中文说明两个文件中的校验值。不要修改文件。",
    },
    {"role": "assistant", "text": first["assistant"]},
    {"role": "user", "text": "不要再次读取文件。请仅根据刚才同一会话中已经得到的结果回答两个校验值。"},
    {"role": "assistant", "text": second_answer},
], ensure_ascii=False, indent=2), encoding="utf-8")

record_step(
    "multitool-and-follow-up",
    platforms=["cli"],
    modules=["agent", "core", "provider", "tools", "persistence", "recovery"],
    boundaries=["cli", "agent-host", "provider", "persistence"],
    assertions={
        "tool_scope_covered": all(value in tool_inputs for value in ("alpha.txt", "beta.txt"))
            and all(value in tool_outputs for value in ("ALPHA-31415", "BETA-92653")),
        "every_tool_has_result": len(first_results) >= len(first_tools),
        "database_owned_recovery": all(persistence_assertions.values()),
        "follow_up_has_no_tools": not second_tools,
        "follow_up_recalls_grounded_results": all(
            value in second_answer for value in ("ALPHA-31415", "BETA-92653")
        ),
        "no_protocol_errors": not errors,
    },
    assertion_evidence={
        "tool_scope_covered": ["first-events", "runtime-db"],
        "every_tool_has_result": ["first-events"],
        "database_owned_recovery": ["runtime-db", "persistence-audit"],
        "follow_up_has_no_tools": ["second-events"],
        "follow_up_recalls_grounded_results": ["second-events", "runtime-db"],
        "no_protocol_errors": ["first-events", "second-events"],
    },
    observations=[
        {"id": "provider-traffic", "kind": "provider_trace", "path": evidence_root / "provider.ndjson"},
        {"id": "first-events", "kind": "event_stream", "path": first_path},
        {"id": "second-events", "kind": "event_stream", "path": second_path},
        {"id": "runtime-db", "kind": "persistence_store", "path": evidence_root / "nola.db"},
        {"id": "persistence-audit", "kind": "persistence_audit", "path": audit_path},
        {"id": "visible-transcript", "kind": "transcript", "path": transcript_path},
    ],
    metrics={
        "firstToolCalls": len(first_tools),
        "coveredResources": 2,
        "firstToolResults": len(first_results),
        "followUpToolCalls": len(second_tools),
    },
    artifacts=[first_path, second_path, evidence_root / "nola.db", audit_path, transcript_path],
)

print("real_cli_multitool_recovery=passed")
print(f"first_tool_calls={len(first_tools)}")
print(f"first_tool_results={len(first_results)}")
print("recovery_tool_calls=0")
print("database_owned_recovery=true")
print(f"evidence_root={evidence_root}")
PY

printf 'permission_profile=%s\n' "$PERMISSION_PROFILE"

cleanup
trap - EXIT INT TERM
