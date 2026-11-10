#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Runs the Android product data-source path on a device with a real configured LLM.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
. "$ROOT/scripts/lib/real_android_device.sh"
CONFIG_PATH="${NOLA_CONFIG_PATH:-$HOME/.nola/config.json}"
JDK_HOME="${NOLA_ANDROID_JAVA_HOME:-/Library/Java/JavaVirtualMachines/temurin-21.jdk/Contents/Home}"
ADB="${ANDROID_ADB:-$HOME/Library/Android/sdk/platform-tools/adb}"
EMULATOR="${ANDROID_EMULATOR:-$HOME/Library/Android/sdk/emulator/emulator}"
LOCAL_CONFIG="$(mktemp "${TMPDIR:-/tmp}/nola-android-product-e2e.XXXXXX")"
RUNTIME_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/nola-android-product-runtime.XXXXXX")"
PROVIDER_TRACE="$RUNTIME_ROOT/provider.ndjson"
PROVIDER_READY="$RUNTIME_ROOT/provider.endpoint"
PROVIDER_PROBE_PID=""
APP_ID="com.nola"
TEST_ID="com.nola.test"

cleanup() {
    if [[ -n "$PROVIDER_PROBE_PID" ]]; then
        kill "$PROVIDER_PROBE_PID" >/dev/null 2>&1 || true
        wait "$PROVIDER_PROBE_PID" >/dev/null 2>&1 || true
    fi
    python3 - "$LOCAL_CONFIG" <<'PY'
import os
import sys
try:
    os.unlink(sys.argv[1])
except FileNotFoundError:
    pass
PY
    if [[ -n "${DEVICE_ID:-}" ]]; then
        "$ADB" -s "$DEVICE_ID" shell run-as "$APP_ID" rm -f \
            files/nola-real-llm-e2e.json \
            files/nola-real-data-source-persistence-audit.json \
            files/nola-real-skill-persistence-audit.json >/dev/null 2>&1 || true
        "$ADB" -s "$DEVICE_ID" shell pm clear "$APP_ID" >/dev/null 2>&1 || true
    fi
    real_android_release_device
    rm -rf "$RUNTIME_ROOT"
}
trap cleanup EXIT INT TERM

[[ -f "$CONFIG_PATH" ]] || { printf 'Missing Nola config: %s\n' "$CONFIG_PATH" >&2; exit 2; }
[[ -x "$JDK_HOME/bin/java" ]] || { printf 'JDK 21 is missing: %s\n' "$JDK_HOME" >&2; exit 2; }
[[ -x "$ADB" ]] || { printf 'adb is missing: %s\n' "$ADB" >&2; exit 2; }

UPSTREAM_ENDPOINT="$(python3 - "$CONFIG_PATH" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as handle:
    llm = json.load(handle).get("llm", {})
print(str(llm.get("endpoint", "")).strip())
PY
)"
[[ -n "$UPSTREAM_ENDPOINT" ]] || { printf 'Nola config is missing llm.endpoint\n' >&2; exit 2; }
HOST_IP="${NOLA_REAL_PROVIDER_HOST_IP:-$(ipconfig getifaddr en0 2>/dev/null || true)}"
if [[ -z "$HOST_IP" ]]; then
    HOST_IP="$(ifconfig | awk '/inet / && $2 !~ /^127\./ && $2 !~ /^169\.254\./ {print $2; exit}')"
fi
[[ -n "$HOST_IP" ]] || { printf 'No private host address is available for the Android provider probe\n' >&2; exit 2; }
python3 "$ROOT/scripts/e2e/provider_probe.py" \
    --upstream "$UPSTREAM_ENDPOINT" \
    --log "$PROVIDER_TRACE" \
    --ready "$PROVIDER_READY" \
    --host "$HOST_IP" \
    >"$RUNTIME_ROOT/provider-probe.log" 2>&1 &
PROVIDER_PROBE_PID=$!
for _ in $(seq 1 100); do
    [[ -s "$PROVIDER_READY" ]] && break
    kill -0 "$PROVIDER_PROBE_PID" 2>/dev/null || break
    sleep 0.05
done
[[ -s "$PROVIDER_READY" ]] || { cat "$RUNTIME_ROOT/provider-probe.log" >&2; exit 2; }
PROVIDER_ENDPOINT="$(python3 - "$PROVIDER_READY" <<'PY'
import json, sys
print(json.load(open(sys.argv[1], encoding="utf-8"))["endpoint"])
PY
)"

python3 - "$CONFIG_PATH" "$LOCAL_CONFIG" "$PROVIDER_ENDPOINT" <<'PY'
import json
import os
import sys
with open(sys.argv[1], encoding="utf-8") as handle:
    llm = json.load(handle).get("llm", {})
result = {
    "endpoint": sys.argv[3],
    "apiKey": str(llm.get("api_key", "")).strip(),
    "model": str(llm.get("model", "")).strip(),
}
if not all(result.values()):
    raise SystemExit("Nola config must define llm.endpoint, llm.api_key, and llm.model")
with open(sys.argv[2], "w", encoding="utf-8") as handle:
    json.dump(result, handle, separators=(",", ":"))
os.chmod(sys.argv[2], 0o600)
PY

REAL_ANDROID_ADB="$ADB"
REAL_ANDROID_EMULATOR="$EMULATOR"
real_android_require_device
DEVICE_ID="$REAL_ANDROID_DEVICE_ID"
export NOLA_ANDROID_ABIS="$REAL_ANDROID_ABI"

printf '[real-android-product] building debug app and instrumentation test\n'
(
    cd "$ROOT/apps/android"
    JAVA_HOME="$JDK_HOME" ./gradlew --quiet :app:assembleDebug :app:assembleDebugAndroidTest
)

APP_APK="$ROOT/apps/android/app/build/outputs/apk/debug/app-debug.apk"
TEST_APK="$ROOT/apps/android/app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk"
[[ -f "$APP_APK" && -f "$TEST_APK" ]] || { printf 'Android test APKs were not produced.\n' >&2; exit 2; }

printf '[real-android-product] installing isolated debug artifacts on %s\n' "$DEVICE_ID"
"$ADB" -s "$DEVICE_ID" install -r -t "$APP_APK" >/dev/null
"$ADB" -s "$DEVICE_ID" install -r -t "$TEST_APK" >/dev/null
"$ADB" -s "$DEVICE_ID" shell run-as "$APP_ID" mkdir -p files
"$ADB" -s "$DEVICE_ID" shell run-as "$APP_ID" rm -f \
    files/nola-real-data-source-persistence-audit.json \
    files/nola-real-skill-persistence-audit.json
"$ADB" -s "$DEVICE_ID" shell "run-as $APP_ID sh -c 'cat > files/nola-real-llm-e2e.json'" <"$LOCAL_CONFIG"

printf '[real-android-product] running Repository -> Core -> Provider -> real LLM\n'
OUTPUT="$(mktemp "${TMPDIR:-/tmp}/nola-android-product-output.XXXXXX")"
set +e
"$ADB" -s "$DEVICE_ID" shell am instrument -w -r \
    -e class com.nola.data.repository.AndroidProductDataSourceRealEndToEndTest \
    "$TEST_ID/androidx.test.runner.AndroidJUnitRunner" | tee "$OUTPUT"
INSTRUMENT_STATUS=${PIPESTATUS[0]}
set -e
ANDROID_PERSISTENCE_STORE="$RUNTIME_ROOT/memory_profile.preferences_pb"
"$ADB" -s "$DEVICE_ID" exec-out run-as "$APP_ID" \
    cat files/datastore/memory_profile.preferences_pb >"$ANDROID_PERSISTENCE_STORE" 2>/dev/null || true
DATA_SOURCE_AUDIT="$RUNTIME_ROOT/data-source-persistence-audit.json"
SKILL_AUDIT="$RUNTIME_ROOT/skill-persistence-audit.json"
"$ADB" -s "$DEVICE_ID" exec-out run-as "$APP_ID" \
    cat files/nola-real-data-source-persistence-audit.json >"$DATA_SOURCE_AUDIT"
"$ADB" -s "$DEVICE_ID" exec-out run-as "$APP_ID" \
    cat files/nola-real-skill-persistence-audit.json >"$SKILL_AUDIT"
python3 - "$ROOT" "$OUTPUT" "$PROVIDER_TRACE" "$ANDROID_PERSISTENCE_STORE" "$DATA_SOURCE_AUDIT" "$SKILL_AUDIT" "$INSTRUMENT_STATUS" <<'PY'
from pathlib import Path
import json
import sys

root, output, provider_trace, persistence_store, data_source_audit, skill_audit, instrument_status = sys.argv[1:]
sys.path.insert(0, str(Path(root) / "scripts"))
from real_e2e_evidence import record_step

text = Path(output).read_text(encoding="utf-8", errors="replace")
instrumentation_passed = int(instrument_status) == 0 and "OK (2 tests)" in text and "FAILURES!!!" not in text
provider_rows = []
for line in Path(provider_trace).read_text(encoding="utf-8", errors="replace").splitlines():
    try:
        row = json.loads(line)
    except json.JSONDecodeError:
        continue
    if row.get("event") == "provider.response" and 200 <= int(row.get("status", 0)) < 300:
        provider_rows.append(row)
real_provider_passed = len(provider_rows) >= 2

record_step(
    "android-device-data-source-skill",
    platforms=["android"],
    modules=["chat", "agent", "core", "provider", "data-source", "skills", "persistence"],
    boundaries=["android-client-runtime", "android-repository", "core", "provider", "persistence"],
    assertions={
        "instrumentation_passed": instrumentation_passed,
        "data_source_chain_passed": instrumentation_passed,
        "skill_chain_passed": instrumentation_passed,
        "real_provider_observed": real_provider_passed,
    },
    assertion_evidence={
        "instrumentation_passed": ["android-instrumentation"],
        "data_source_chain_passed": ["android-instrumentation", "android-provider-traffic"],
        "skill_chain_passed": ["android-instrumentation", "android-provider-traffic"],
        "real_provider_observed": ["android-provider-traffic"],
    },
    observations=[
        {"id": "android-instrumentation", "kind": "instrumentation_output", "path": output},
        {"id": "android-provider-traffic", "kind": "provider_trace", "path": provider_trace},
        {"id": "android-persistence-store", "kind": "persistence_store", "path": persistence_store},
        {"id": "android-data-source-audit", "kind": "persistence_audit", "path": data_source_audit},
        {"id": "android-skill-audit", "kind": "persistence_audit", "path": skill_audit},
    ],
    metrics={"instrumentExitCode": int(instrument_status), "observedProviderResponses": len(provider_rows)},
)
if not instrumentation_passed or not real_provider_passed:
    raise SystemExit("Android product data-source and Skill lifecycle E2E failed")
PY
printf '[real-android-product] passed\n'
