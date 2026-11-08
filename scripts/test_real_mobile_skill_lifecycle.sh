#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Real-model uploaded-Skill lifecycle acceptance for iOS and Android.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
. "$ROOT/scripts/lib/real_android_device.sh"
. "$ROOT/scripts/lib/xcode_test_device_guard.sh"
CONFIG_PATH="${NOLA_CONFIG_PATH:-$HOME/.nola/config.json}"
ANDROID_SDK="${ANDROID_SDK_ROOT:-${ANDROID_HOME:-$HOME/Library/Android/sdk}}"
ADB="$ANDROID_SDK/platform-tools/adb"
EMULATOR="$ANDROID_SDK/emulator/emulator"
JDK_HOME="${NOLA_ANDROID_JAVA_HOME:-/Library/Java/JavaVirtualMachines/temurin-21.jdk/Contents/Home}"
ANDROID_CONFIG="$(mktemp /tmp/nola-real-mobile-skill.XXXXXX.json)"
ANDROID_RESULT="$(mktemp /tmp/nola-real-mobile-skill-android.XXXXXX.log)"
EVIDENCE_WORK_DIR="$(mktemp -d /tmp/nola-real-mobile-skill-evidence.XXXXXX)"
PROVIDER_TRACE="$EVIDENCE_WORK_DIR/provider.ndjson"
PROVIDER_READY="$EVIDENCE_WORK_DIR/provider-ready.json"
PROVIDER_LOG="$EVIDENCE_WORK_DIR/provider-probe.log"
IOS_RESULT_REPORT="$EVIDENCE_WORK_DIR/ios-test-report.json"
PROBE_PID=""
IOS_DERIVED_DATA_WAS_CREATED=0
if [[ -n "${NOLA_IOS_SKILL_DERIVED_DATA:-}" ]]; then
    IOS_DERIVED_DATA="$NOLA_IOS_SKILL_DERIVED_DATA"
    mkdir -p "$IOS_DERIVED_DATA"
elif [[ "${NOLA_IOS_REAL_CLEAN_BUILD:-0}" == "1" ]]; then
    IOS_DERIVED_DATA="$(mktemp -d /tmp/nola-ios-real-skill-derived-data.XXXXXX)"
    IOS_DERIVED_DATA_WAS_CREATED=1
else
    IOS_DERIVED_DATA="${NOLA_REAL_E2E_CACHE_ROOT:-$ROOT/.artifacts/real-e2e-cache}/ios-skill-derived-data"
    mkdir -p "$IOS_DERIVED_DATA"
fi
STARTED_ANDROID_EMULATOR=0
ANDROID_SERIAL=""
ANDROID_REVERSE_PORT=""

cleanup() {
    local status=$?
    if [[ -n "$PROBE_PID" ]]; then
        kill "$PROBE_PID" >/dev/null 2>&1 || true
        wait "$PROBE_PID" >/dev/null 2>&1 || true
    fi
    if [[ -n "$ANDROID_SERIAL" ]]; then
        if [[ -n "$ANDROID_REVERSE_PORT" ]]; then
            "$ADB" -s "$ANDROID_SERIAL" reverse --remove "tcp:$ANDROID_REVERSE_PORT" >/dev/null 2>&1 || true
        fi
        "$ADB" -s "$ANDROID_SERIAL" shell run-as com.nola rm -f \
            /data/user/0/com.nola/files/nola-real-llm-e2e.json >/dev/null 2>&1 || true
        "$ADB" -s "$ANDROID_SERIAL" shell rm -f \
            /data/local/tmp/nola-real-llm-e2e.json >/dev/null 2>&1 || true
    fi
    real_android_release_device
    rm -f "$ANDROID_CONFIG" "$ANDROID_RESULT"
    if [[ "$IOS_DERIVED_DATA_WAS_CREATED" == "1" && "${NOLA_IOS_REAL_PRESERVE_DERIVED_DATA:-0}" != "1" ]]; then
        rm -rf "$IOS_DERIVED_DATA"
    fi
    rm -rf "$EVIDENCE_WORK_DIR"
    nola_xctest_device_guard_end "$ROOT" ios_simulator
    return "$status"
}
trap cleanup EXIT INT TERM

[[ -f "$CONFIG_PATH" ]] || { printf 'Missing Nola config: %s\n' "$CONFIG_PATH" >&2; exit 2; }
[[ -x "$ADB" ]] || { printf 'adb is missing: %s\n' "$ADB" >&2; exit 2; }
[[ -x "$JDK_HOME/bin/java" ]] || { printf 'JDK is missing: %s\n' "$JDK_HOME" >&2; exit 2; }

UPSTREAM_ENDPOINT="$(python3 - "$CONFIG_PATH" <<'PY'
import json
import os
import sys
with open(sys.argv[1], encoding="utf-8") as handle:
    configured = str(json.load(handle).get("llm", {}).get("endpoint", "")).strip()
print(os.environ.get("NOLA_REAL_LLM_ENDPOINT_OVERRIDE", configured).strip())
PY
)"
[[ -n "$UPSTREAM_ENDPOINT" ]] || { printf 'Nola config must define llm.endpoint\n' >&2; exit 2; }
python3 "$ROOT/scripts/e2e/provider_probe.py" \
    --upstream "$UPSTREAM_ENDPOINT" \
    --log "$PROVIDER_TRACE" \
    --ready "$PROVIDER_READY" \
    >"$PROVIDER_LOG" 2>&1 &
PROBE_PID=$!
for _ in {1..100}; do
    [[ -s "$PROVIDER_READY" ]] && break
    kill -0 "$PROBE_PID" 2>/dev/null || {
        cat "$PROVIDER_LOG" >&2
        exit 2
    }
    sleep 0.1
done
[[ -s "$PROVIDER_READY" ]] || { cat "$PROVIDER_LOG" >&2; exit 2; }
export NOLA_REAL_LLM_ENDPOINT_OVERRIDE="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["endpoint"])' "$PROVIDER_READY")"

python3 - "$CONFIG_PATH" "$ANDROID_CONFIG" <<'PY'
import json
import os
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    llm = json.load(handle).get("llm", {})
values = {
    "endpoint": os.environ.get("NOLA_REAL_LLM_ENDPOINT_OVERRIDE", str(llm.get("endpoint", ""))).strip(),
    "apiKey": str(llm.get("api_key", "")).strip(),
    "model": str(llm.get("model", "")).strip(),
}
if not all(values.values()):
    raise SystemExit("Nola config must define llm.endpoint, llm.api_key, and llm.model")
with open(sys.argv[2], "w", encoding="utf-8") as handle:
    json.dump(values, handle)
os.chmod(sys.argv[2], 0o600)
PY

if [[ "${NOLA_REAL_SKIP_ANDROID:-0}" != "1" ]]; then
    REAL_ANDROID_ADB="$ADB"
    REAL_ANDROID_EMULATOR="$EMULATOR"
    real_android_require_device
    ANDROID_SERIAL="$REAL_ANDROID_DEVICE_ID"
    STARTED_ANDROID_EMULATOR="$REAL_ANDROID_STARTED_EMULATOR"
    ANDROID_ABI="$REAL_ANDROID_ABI"
    export NOLA_ANDROID_ABIS="$ANDROID_ABI"

    ANDROID_REVERSE_PORT="$(python3 - "$ANDROID_CONFIG" <<'PY'
import json
import sys
from urllib.parse import urlparse
with open(sys.argv[1], encoding="utf-8") as handle:
    endpoint = str(json.load(handle).get("endpoint", ""))
parsed = urlparse(endpoint)
if parsed.hostname in {"127.0.0.1", "localhost", "::1"}:
    print(parsed.port or (443 if parsed.scheme == "https" else 80))
PY
)"
    if [[ -n "$ANDROID_REVERSE_PORT" ]]; then
        "$ADB" -s "$ANDROID_SERIAL" reverse \
            "tcp:$ANDROID_REVERSE_PORT" "tcp:$ANDROID_REVERSE_PORT"
    fi

    printf '[real-mobile-skill] building Android product and instrumentation for %s\n' "$ANDROID_ABI"
    (
        cd "$ROOT/android"
        JAVA_HOME="$JDK_HOME" ./gradlew \
            :app:assembleDebug :app:assembleDebugAndroidTest --no-daemon --quiet
    )
    "$ADB" -s "$ANDROID_SERIAL" install -r \
        "$ROOT/android/app/build/outputs/apk/debug/app-debug.apk" >/dev/null
    "$ADB" -s "$ANDROID_SERIAL" install -r \
        "$ROOT/android/app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk" >/dev/null
    "$ADB" -s "$ANDROID_SERIAL" push "$ANDROID_CONFIG" \
        /data/local/tmp/nola-real-llm-e2e.json >/dev/null
    "$ADB" -s "$ANDROID_SERIAL" shell run-as com.nola mkdir -p /data/user/0/com.nola/files
    "$ADB" -s "$ANDROID_SERIAL" shell run-as com.nola cp \
        /data/local/tmp/nola-real-llm-e2e.json \
        /data/user/0/com.nola/files/nola-real-llm-e2e.json
    "$ADB" -s "$ANDROID_SERIAL" shell am instrument -w -r \
        -e class 'com.nola.data.repository.AndroidProductDataSourceRealEndToEndTest#uploadedSkillLifecycleFlowsThroughStoreRepositoryCoreAndRealProvider' \
        com.nola.test/androidx.test.runner.AndroidJUnitRunner | tee "$ANDROID_RESULT"
    rg -q '^OK \(1 test\)$' "$ANDROID_RESULT"
    if rg -q 'FAILURES!!!|INSTRUMENTATION_STATUS_CODE: -2' "$ANDROID_RESULT"; then
        printf 'Android Skill lifecycle failed\n' >&2
        exit 9
    fi
    printf '[real-mobile-skill] Android passed\n'
fi

if [[ "${NOLA_REAL_SKIP_IOS:-0}" == "1" ]]; then
    exit 0
fi

nola_xctest_device_guard_begin "$ROOT" ios_simulator
IOS_DEVICE_ID="${NOLA_IOS_REAL_DEVICE_ID:-}"
if [[ -z "$IOS_DEVICE_ID" ]]; then
    IOS_DEVICE_ID="$(xcrun simctl list devices available -j | python3 -c '
import json, sys
items = [item for values in json.load(sys.stdin).get("devices", {}).values()
         for item in values if item.get("isAvailable") and "iPhone" in item.get("name", "")]
selected = next((item for item in items if item.get("state") == "Booted"), None)
selected = selected or (items[0] if items else None)
if not selected: raise SystemExit("No available iPhone simulator")
print(selected["udid"])
')"
fi
xcrun simctl boot "$IOS_DEVICE_ID" >/dev/null 2>&1 || true
xcrun simctl bootstatus "$IOS_DEVICE_ID" -b >/dev/null

printf '[real-mobile-skill] building iOS product tests\n'
xcodebuild build-for-testing -project "$ROOT/Nola.xcodeproj" -scheme Nola \
    -destination "platform=iOS Simulator,id=$IOS_DEVICE_ID" \
    -derivedDataPath "$IOS_DERIVED_DATA" CODE_SIGNING_ALLOWED=NO -quiet
XCTESTRUN="$(find "$IOS_DERIVED_DATA/Build/Products" -maxdepth 1 -name '*.xctestrun' -print -quit)"
[[ -f "$XCTESTRUN" ]] || { printf 'iOS xctestrun is missing\n' >&2; exit 2; }

python3 - "$CONFIG_PATH" "$XCTESTRUN" "$IOS_DEVICE_ID" "$IOS_RESULT_REPORT" <<'PY'
import json
import os
import plistlib
import shutil
import subprocess
import sys
import tempfile

config_path, source_path, device_id, report_path = sys.argv[1:]
with open(config_path, encoding="utf-8") as handle:
    llm = json.load(handle).get("llm", {})
values = {key: str(llm.get(key, "")).strip() for key in ("endpoint", "api_key", "model")}
values["endpoint"] = os.environ.get("NOLA_REAL_LLM_ENDPOINT_OVERRIDE", values["endpoint"]).strip()
if not all(values.values()):
    raise SystemExit("Incomplete real LLM configuration")
descriptor, run_path = tempfile.mkstemp(prefix="Nola_real_skill_", suffix=".xctestrun", dir=os.path.dirname(source_path))
os.close(descriptor)
result_directory = tempfile.mkdtemp(prefix="nola-ios-real-skill-result-", dir="/tmp")
result_path = os.path.join(result_directory, "result.xcresult")
try:
    with open(source_path, "rb") as handle:
        run = plistlib.load(handle)
    found = False
    for configuration in run.get("TestConfigurations", []):
        for target in configuration.get("TestTargets", []):
            if target.get("BlueprintName") != "NolaTests":
                continue
            target.setdefault("EnvironmentVariables", {}).update({
                "NOLA_REAL_LLM_E2E": "1",
                "NOLA_REAL_LLM_ENDPOINT": values["endpoint"],
                "NOLA_REAL_LLM_API_KEY": values["api_key"],
                "NOLA_REAL_LLM_MODEL": values["model"],
            })
            found = True
    if not found:
        raise SystemExit("NolaTests target missing from xctestrun")
    with open(run_path, "wb") as handle:
        plistlib.dump(run, handle)
    completed = subprocess.run([
        "xcodebuild", "test-without-building", "-xctestrun", run_path,
        "-destination", f"platform=iOS Simulator,id={device_id}",
        "-parallel-testing-enabled", "NO",
        "-only-testing:NolaTests/EndToEndFlowTests/testRealModelObservesUploadedSkillEnableEditAndDisableLifecycle",
        "-resultBundlePath", result_path, "-quiet",
    ], check=False)
    summary = subprocess.run([
        "xcrun", "xcresulttool", "get", "test-results", "summary", "--path", result_path,
    ], check=True, capture_output=True, text=True)
    report = json.loads(summary.stdout)
    with open(report_path, "w", encoding="utf-8") as destination:
        json.dump(report, destination, ensure_ascii=False, indent=2)
    print("[real-mobile-skill] iOS report", {
        "total": report.get("totalTestCount"),
        "passed": report.get("passedTests"),
        "failed": report.get("failedTests"),
        "skipped": report.get("skippedTests"),
    })
    if completed.returncode or report.get("totalTestCount") != 1 or report.get("passedTests") != 1 or report.get("skippedTests") != 0:
        raise SystemExit(completed.returncode or 9)
finally:
    try: os.unlink(run_path)
    except FileNotFoundError: pass
    shutil.rmtree(result_directory, ignore_errors=True)
PY
printf '[real-mobile-skill] iOS passed\n'

python3 - "$ROOT" "$ANDROID_RESULT" "$PROVIDER_TRACE" "$IOS_RESULT_REPORT" <<'PY'
import os
from pathlib import Path
import sys

root, android_result, provider_trace, ios_report = sys.argv[1:]
sys.path.insert(0, str(Path(root) / "scripts"))
from real_e2e_evidence import record_step

if os.environ.get("NOLA_REAL_SKIP_ANDROID") != "1":
    record_step(
        "android-skill-enable-edit-disable",
        platforms=["android"],
        modules=["chat", "agent", "core", "provider", "skills"],
        boundaries=["android-client-runtime", "provider"],
        assertions={"skill_lifecycle_passed": True},
        assertion_evidence={"skill_lifecycle_passed": ["android-instrumentation", "provider-traffic"]},
        observations=[
            {"id": "android-instrumentation", "kind": "instrumentation_output", "path": android_result},
            {"id": "provider-traffic", "kind": "provider_trace", "path": provider_trace},
        ],
    )
if os.environ.get("NOLA_REAL_SKIP_IOS") != "1":
    record_step(
        "ios-skill-enable-edit-disable",
        platforms=["ios"],
        modules=["chat", "agent", "core", "provider", "skills"],
        boundaries=["ios-client-runtime", "provider"],
        assertions={"skill_lifecycle_passed": True},
        assertion_evidence={"skill_lifecycle_passed": ["ios-test-report", "provider-traffic"]},
        observations=[
            {"id": "ios-test-report", "kind": "test_report", "path": ios_report},
            {"id": "provider-traffic", "kind": "provider_trace", "path": provider_trace},
        ],
    )
PY
