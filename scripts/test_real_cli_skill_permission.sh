#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Real-provider CLI permission, recovery, persistence, and Skill chain.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
source "$ROOT/scripts/e2e/cargo_paths.sh"
PROFILE="${NOLA_REAL_CLI_PROFILE:-debug}"
case "$PROFILE" in
    debug|release) ;;
    *) printf 'Unsupported NOLA_REAL_CLI_PROFILE: %s\n' "$PROFILE" >&2; exit 2 ;;
esac
BINARY="${NOLA_REAL_CLI_BINARY:-$(nola_cargo_binary "$ROOT" "$PROFILE" nola)}"
if [[ "${NOLA_REAL_E2E_PREFLIGHT_ONLY:-0}" == "1" ]]; then
    printf '{"profile":"%s","binary":"%s","buildMode":"%s"}\n' \
        "$PROFILE" "$BINARY" "$PROFILE"
    exit 0
fi
RUNTIME_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/nola-real-cli-skill-permission.XXXXXX")"
PROBE_PID=""

cleanup() {
    local status=$?
    if [[ -n "$PROBE_PID" ]]; then
        kill "$PROBE_PID" 2>/dev/null || true
        wait "$PROBE_PID" 2>/dev/null || true
    fi
    # The registered Service/DB and raw evidence remain inspectable on success
    # and failure. Never unlink a live registered database during teardown.
    printf '{"preservedRuntime":"%s","exit":%s}\n' "$RUNTIME_ROOT" "$status"
    return "$status"
}
trap cleanup EXIT

resolve_real_e2e_model() {
    if [[ -n "${NOLA_REAL_E2E_MODEL:-}" ]]; then
        printf '%s\n' "$NOLA_REAL_E2E_MODEL"
        return 0
    fi
    if [[ -n "${MODEL_NAME:-}" ]]; then
        printf '%s\n' "$MODEL_NAME"
        return 0
    fi

    local default_database="$HOME/Library/Application Support/Nola/State/nola-v4.db"
    local database="${NOLA_MODEL_PROFILE_DB:-$default_database}"
    if [[ -f "$database" ]] && command -v sqlite3 >/dev/null 2>&1; then
        sqlite3 "$database" \
            "SELECT model_name FROM model_profiles
             WHERE selected=1 AND enabled=1 AND deleted_at_ms IS NULL AND runtime='remote'
             ORDER BY updated_at_ms DESC LIMIT 1;" 2>/dev/null
    fi
}

resolve_real_e2e_endpoint() {
    if [[ -n "${NOLA_REAL_E2E_ENDPOINT:-}" ]]; then
        printf '%s\n' "$NOLA_REAL_E2E_ENDPOINT"
        return 0
    fi
    if [[ -n "${MODEL_API_URL:-}" ]]; then
        printf '%s\n' "$MODEL_API_URL"
        return 0
    fi

    local default_database="$HOME/Library/Application Support/Nola/State/nola-v4.db"
    local database="${NOLA_MODEL_PROFILE_DB:-$default_database}"
    local endpoint=""
    if [[ -f "$database" ]] && command -v sqlite3 >/dev/null 2>&1; then
        endpoint="$(sqlite3 "$database" \
            "SELECT json_extract(source_json,'$.baseURL')
             FROM model_profiles
             WHERE selected=1 AND enabled=1 AND deleted_at_ms IS NULL AND runtime='remote'
             ORDER BY updated_at_ms DESC LIMIT 1;" 2>/dev/null)"
    fi
    endpoint="${endpoint%/}"
    endpoint="${endpoint%/chat/completions}"
    printf '%s\n' "$endpoint"
}

MODEL_SOURCE="nola-store"
if [[ -n "${MODEL_NAME:-}" || -n "${MODEL_API_URL:-}" ]]; then
    MODEL_SOURCE="environment"
fi
if [[ -n "${NOLA_REAL_E2E_MODEL:-}" || -n "${NOLA_REAL_E2E_ENDPOINT:-}" ]]; then
    MODEL_SOURCE="explicit"
fi

MODEL_NAME="$(resolve_real_e2e_model)"
if [[ -z "$MODEL_NAME" ]]; then
    printf '%s\n' \
        'No selected remote Nola model was found. Set NOLA_REAL_E2E_MODEL or NOLA_MODEL_PROFILE_DB.' >&2
    exit 2
fi
MODEL_API_URL="$(resolve_real_e2e_endpoint)"
if [[ -z "$MODEL_API_URL" ]]; then
    printf '%s\n' \
        'No endpoint for the selected remote Nola model was found. Set NOLA_REAL_E2E_ENDPOINT.' >&2
    exit 2
fi
MODEL_API_KEY="${NOLA_REAL_E2E_API_KEY:-${MODEL_API_KEY:-${OPENROUTER_API_KEY:-}}}"
if [[ -z "$MODEL_API_KEY" ]]; then
    printf '%s\n' \
        'No real-provider credential was found. Set NOLA_REAL_E2E_API_KEY or MODEL_API_KEY.' >&2
    exit 2
fi
printf '{"stage":"model-selection","model":"%s","endpoint":"%s","source":"%s"}\n' \
    "$MODEL_NAME" "$MODEL_API_URL" "$MODEL_SOURCE"
if [[ "${NOLA_REAL_E2E_MODEL_SELECTION_ONLY:-0}" == "1" ]]; then
    exit 0
fi
if [[ "${NOLA_SKIP_CLI_BUILD:-0}" != "1" ]]; then
    BUILD_STARTED=$SECONDS
    if [[ "$PROFILE" == "release" ]]; then
        cargo build --release -p nola_cli --bin nola --example terminal_acceptance_oracle -p nola_service --bin nola-service --manifest-path "$ROOT/apps/cli/Cargo.toml"
    else
        cargo build -p nola_cli --bin nola --example terminal_acceptance_oracle -p nola_service --bin nola-service --manifest-path "$ROOT/apps/cli/Cargo.toml"
    fi
    printf '{"stage":"build","profile":"%s","durationSeconds":%s}\n' \
        "$PROFILE" "$((SECONDS - BUILD_STARTED))"
fi
[[ -x "$BINARY" ]] || { printf 'Missing CLI binary: %s\n' "$BINARY" >&2; exit 2; }
ORACLE="${NOLA_REAL_CLI_ORACLE:-$(nola_cargo_example "$ROOT" "$PROFILE" terminal_acceptance_oracle "$ROOT/apps/cli/Cargo.toml")}"
[[ -x "$ORACLE" ]] || { printf 'Missing terminal oracle: %s\n' "$ORACLE" >&2; exit 2; }

mkdir -p "$RUNTIME_ROOT/home/.nola" "$RUNTIME_ROOT/project" "$RUNTIME_ROOT/evidence"
UPSTREAM="$MODEL_API_URL"
python3 "$ROOT/scripts/e2e/provider_probe.py" \
    --upstream "$UPSTREAM" \
    --upstream-timeout-seconds "${NOLA_REAL_PROVIDER_TIMEOUT_SECONDS:-45}" \
    --log "$RUNTIME_ROOT/evidence/provider.ndjson" \
    --ready "$RUNTIME_ROOT/evidence/provider-ready.json" \
    >"$RUNTIME_ROOT/evidence/provider-probe.log" 2>&1 &
PROBE_PID=$!
for _ in {1..100}; do
    [[ -s "$RUNTIME_ROOT/evidence/provider-ready.json" ]] && break
    kill -0 "$PROBE_PID" 2>/dev/null || { cat "$RUNTIME_ROOT/evidence/provider-probe.log" >&2; exit 2; }
    sleep 0.1
done
[[ -s "$RUNTIME_ROOT/evidence/provider-ready.json" ]] || { echo 'Provider probe did not become ready.' >&2; exit 2; }

export NOLA_HOME="$RUNTIME_ROOT/home"
export NOLA_MAC_APP_SUPPORT="$RUNTIME_ROOT/app-support"
export NOLA_EXTERNAL_EXECUTION_HOME="$RUNTIME_ROOT/execution-home"
mkdir -p "$NOLA_MAC_APP_SUPPORT" "$NOLA_EXTERNAL_EXECUTION_HOME"
export NOLA_SKIP_ONBOARDING=1
export NOLA_REAL_RESILIENCE_PROJECT="$RUNTIME_ROOT/project"
export NOLA_REAL_TEST_ENDPOINT="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["endpoint"])' "$RUNTIME_ROOT/evidence/provider-ready.json")"
export NOLA_REAL_TEST_MODEL="$MODEL_NAME"
export NOLA_REAL_TEST_API_KEY="$MODEL_API_KEY"
export NOLA_REAL_TEST_PROVIDER_LOG="$RUNTIME_ROOT/evidence/provider.ndjson"
python3 "$ROOT/scripts/test_real_cli_resilience.py" "$BINARY" "$ORACLE" \
    | tee "$RUNTIME_ROOT/evidence/resilience-report.jsonl"

if [[ -n "${NOLA_REAL_TEST_ONLY_CASE:-}" ]]; then
    python3 - "$RUNTIME_ROOT/evidence/resilience-report.jsonl" "$RUNTIME_ROOT/evidence/cli-audit.json" "$NOLA_REAL_TEST_ONLY_CASE" <<'PY'
import json, sys
report, output, expected_case = sys.argv[1:]
cases = {}
for line in open(report, encoding="utf-8"):
    try:
        item = json.loads(line)
    except json.JSONDecodeError:
        continue
    if item.get("case"):
        cases[item["case"]] = item
selected = cases.get(expected_case)
completed = cases.get("complete")
payload = {
    "passed": bool(selected and completed and completed.get("passed")),
    "targetedCase": expected_case,
    "cases": sorted(cases),
}
json.dump(payload, open(output, "w", encoding="utf-8"), ensure_ascii=False, indent=2)
if not payload["passed"]:
    raise SystemExit(9)
PY
    printf '[real-cli-skill-permission] targeted case passed: %s\n' "$NOLA_REAL_TEST_ONLY_CASE"
    exit 0
fi

export NOLA_RUN_LIVE_SKILL_E2E=1
export NOLA_SKIP_CLI_BUILD=1
export NOLA_REAL_CLI_BINARY="$BINARY"
# The live Skill harness uses NOLA_BIN as its canonical executable override.
# Reuse the binary already selected/built by this wrapper instead of resolving
# Cargo metadata or compiling a second artifact mid-scenario.
export NOLA_BIN="$BINARY"
export NOLA_LIVE_SKILL_ORACLE="$ORACLE"
export NOLA_LIVE_SKILL_ENDPOINT="$NOLA_REAL_TEST_ENDPOINT"
export NOLA_LIVE_SKILL_MODEL="$MODEL_NAME"
export NOLA_LIVE_SKILL_API_KEY="$MODEL_API_KEY"
python3 "$ROOT/scripts/live_skill_e2e.py" analyze-data \
    | tee "$RUNTIME_ROOT/evidence/skill-report.log"

python3 - "$RUNTIME_ROOT/evidence/resilience-report.jsonl" "$RUNTIME_ROOT/evidence/skill-report.log" "$RUNTIME_ROOT/evidence/cli-audit.json" <<'PY'
import json, sys
resilience, skill, output = sys.argv[1:]
cases = {}
for line in open(resilience, encoding="utf-8"):
    try:
        item = json.loads(line)
    except json.JSONDecodeError:
        continue
    if item.get("case"):
        cases[item["case"]] = item
skill_text = open(skill, encoding="utf-8").read()
required = {
    "provider_ready": bool(cases.get("provider-ready", {}).get("passed")),
    "permission_custom_hung_then_replaced": bool(cases.get("permission-custom-response", {}).get("modelFollowedReplacement")),
    "allow_once_prompted_again": bool(cases.get("allow-once-prompts-again", {}).get("secondPromptShown")) and bool(cases.get("allow-once-prompts-again", {}).get("secondSideEffectPrevented")),
    "deny_prevented_substitution": cases.get("deny-no-substitution", {}).get("bashSubstitution") is False and bool(cases.get("deny-no-substitution", {}).get("sideEffectPrevented")),
    "read_only_needed_no_permission": cases.get("read-only-no-permission", {}).get("permissionRequested") is False and bool(cases.get("read-only-no-permission", {}).get("fixtureUnchanged")),
    "ambiguous_words_did_not_force_tools": cases.get("multilingual-ambiguous-no-forced-tools", {}).get("toolCount") == 0 and bool(cases.get("multilingual-ambiguous-no-forced-tools", {}).get("naturalFollowUpCompleted")),
    "session_permission_survived_restart": cases.get("session-permission-restart", {}).get("promptedAfterRestart") is False,
    "cancel_recorded_without_failure": bool(cases.get("escape-cancelled-turn", {}).get("cancelledTerminal")) and cases.get("escape-cancelled-turn", {}).get("failedTerminal") is False,
    "goal_return_stopped_successor_runs": cases.get("goal-return-to-composer-stops-successors", {}).get("successorRunCreated") is False and cases.get("goal-return-to-composer-stops-successors", {}).get("providerRequestAfterReturn") is False,
    "failed_turn_recovered_with_auditable_placeholder": bool(cases.get("failed-turn-session-recovery", {}).get("recovered")) and bool(cases.get("failed-turn-session-recovery", {}).get("failedAssistantRecorded")) and bool(cases.get("failed-turn-session-recovery", {}).get("failedAssistantExcludedFromTextContext")),
    "skill_created_validated_artifact": "[PASS] analyze-data" in skill_text,
}
payload = {"passed": all(required.values()), "assertions": required, "cases": sorted(cases)}
json.dump(payload, open(output, "w", encoding="utf-8"), ensure_ascii=False, indent=2)
if not payload["passed"]:
    raise SystemExit(9)
PY

if [[ -n "${NOLA_E2E_RECEIPT_PATH:-}" ]]; then
EVIDENCE_EXPORT_DIR="$(dirname "$NOLA_E2E_RECEIPT_PATH")/cli-runtime-evidence"
mkdir -p "$EVIDENCE_EXPORT_DIR"
cp "$RUNTIME_ROOT/evidence/provider.ndjson" "$EVIDENCE_EXPORT_DIR/provider.ndjson"
cp "$RUNTIME_ROOT/evidence/cli-audit.json" "$EVIDENCE_EXPORT_DIR/cli-audit.json"
cp "$RUNTIME_ROOT/evidence/resilience-report.jsonl" "$EVIDENCE_EXPORT_DIR/resilience-report.jsonl"
cp "$RUNTIME_ROOT/evidence/skill-report.log" "$EVIDENCE_EXPORT_DIR/skill-report.log"
cp "$RUNTIME_ROOT/project/nola-e2e.db" "$EVIDENCE_EXPORT_DIR/nola-e2e.db"
python3 - "$ROOT" "$EVIDENCE_EXPORT_DIR" <<'PY'
import sys
from pathlib import Path
repo, root = Path(sys.argv[1]), Path(sys.argv[2])
sys.path.insert(0, str(repo / "scripts"))
from real_e2e_evidence import record_step
record_step(
    "cli-permission-skill-chain",
    platforms=["cli"],
    modules=["agent", "chat", "core", "permissions", "persistence", "provider", "recovery", "skills", "tools"],
    boundaries=["cli-runtime", "provider", "persistence"],
    assertions={"cli_permission_skill_chain_passed": True},
    assertion_evidence={"cli_permission_skill_chain_passed": ["provider-traffic", "cli-audit", "resilience-report", "skill-report", "cli-runtime-db"]},
    observations=[
        {"id": "provider-traffic", "kind": "provider_trace", "path": root / "provider.ndjson"},
        {"id": "cli-audit", "kind": "persistence_audit", "path": root / "cli-audit.json"},
        {"id": "resilience-report", "kind": "test_report", "path": root / "resilience-report.jsonl"},
        {"id": "skill-report", "kind": "test_report", "path": root / "skill-report.log"},
        {"id": "cli-runtime-db", "kind": "persistence_store", "path": root / "nola-e2e.db"},
    ],
)
PY
fi

printf '[real-cli-skill-permission] passed\n'
