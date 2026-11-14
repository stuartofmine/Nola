#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Real-provider acceptance for iOS and Android provider paths.
# Configuration is read from ~/.nola/config.json and credentials are never printed.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
. "$ROOT/scripts/lib/xcode_test_device_guard.sh"
CONFIG_PATH="${NOLA_CONFIG_PATH:-$HOME/.nola/config.json}"
DERIVED_DATA_WAS_CREATED=0
if [[ -n "${NOLA_IOS_REAL_DERIVED_DATA:-}" ]]; then
    DERIVED_DATA="$NOLA_IOS_REAL_DERIVED_DATA"
    mkdir -p "$DERIVED_DATA"
else
    DERIVED_DATA="$(mktemp -d "${TMPDIR:-/tmp}/nola-ios-real-derived-data.XXXXXX")"
    DERIVED_DATA_WAS_CREATED=1
fi
JDK_HOME="${NOLA_ANDROID_JAVA_HOME:-/Library/Java/JavaVirtualMachines/temurin-21.jdk/Contents/Home}"
MOBILE_PLATFORM="${NOLA_REAL_MOBILE_PLATFORM:-all}"
EVIDENCE_WORK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/nola-real-mobile-evidence.XXXXXX")"
PROBE_PID=""

case "$MOBILE_PLATFORM" in
    all|android|ios) ;;
    *) printf 'NOLA_REAL_MOBILE_PLATFORM must be all, android, or ios.\n' >&2; exit 2 ;;
esac

cleanup() {
    local status=$?
    if [[ -n "$PROBE_PID" ]]; then
        kill "$PROBE_PID" 2>/dev/null || true
        wait "$PROBE_PID" 2>/dev/null || true
    fi
    if [[ "$DERIVED_DATA_WAS_CREATED" == "1" && "${NOLA_IOS_REAL_PRESERVE_DERIVED_DATA:-0}" != "1" ]]; then
        rm -rf "$DERIVED_DATA"
    fi
    rm -rf "$EVIDENCE_WORK_DIR"
    nola_xctest_device_guard_end "$ROOT" ios_simulator
    return "$status"
}
trap cleanup EXIT INT TERM

[[ -f "$CONFIG_PATH" ]] || {
    printf 'Missing Nola config: %s\n' "$CONFIG_PATH" >&2
    exit 2
}
[[ -x "$JDK_HOME/bin/java" ]] || {
    printf 'JDK 21 is missing: %s\n' "$JDK_HOME" >&2
    exit 2
}

read_llm_setting() {
    node - "$CONFIG_PATH" "$1" <<'NODE'
const fs = require("fs");
const value = JSON.parse(fs.readFileSync(process.argv[2], "utf8"));
const item = typeof value?.llm?.[process.argv[3]] === "string"
  ? value.llm[process.argv[3]].trim()
  : "";
if (!item) process.exit(3);
process.stdout.write(item);
NODE
}

export NOLA_REAL_LLM_E2E=1
export NOLA_REAL_LLM_ENDPOINT="$(read_llm_setting endpoint)" || {
    printf 'Nola config must define llm.endpoint, llm.api_key, and llm.model.\n' >&2
    exit 2
}
export NOLA_REAL_LLM_API_KEY="$(read_llm_setting api_key)" || exit 2
export NOLA_REAL_LLM_MODEL="$(read_llm_setting model)" || exit 2
UPSTREAM_ENDPOINT="$NOLA_REAL_LLM_ENDPOINT"

start_provider_probe() {
    local platform="$1"
    local ready="$EVIDENCE_WORK_DIR/$platform-provider-ready.json"
    local trace="$EVIDENCE_WORK_DIR/$platform-provider.ndjson"
    rm -f "$ready" "$trace"
    python3 "$ROOT/scripts/e2e/provider_probe.py" \
        --upstream "$UPSTREAM_ENDPOINT" \
        --log "$trace" \
        --ready "$ready" \
        >"$EVIDENCE_WORK_DIR/$platform-provider-probe.log" 2>&1 &
    PROBE_PID=$!
    for _ in {1..100}; do
        [[ -s "$ready" ]] && break
        kill -0 "$PROBE_PID" 2>/dev/null || {
            cat "$EVIDENCE_WORK_DIR/$platform-provider-probe.log" >&2
            return 1
        }
        sleep 0.1
    done
    [[ -s "$ready" ]] || return 1
    NOLA_REAL_LLM_ENDPOINT="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["endpoint"])' "$ready")"
    export NOLA_REAL_LLM_ENDPOINT
}

stop_provider_probe() {
    if [[ -n "$PROBE_PID" ]]; then
        kill "$PROBE_PID" 2>/dev/null || true
        wait "$PROBE_PID" 2>/dev/null || true
        PROBE_PID=""
    fi
}

ANDROID_REPORT="$ROOT/apps/android/app/build/test-results/testDebugUnitTest/TEST-com.nola.llm.provider.OpenAiProviderRealEndToEndTest.xml"
if [[ "$MOBILE_PLATFORM" == "all" || "$MOBILE_PLATFORM" == "android" ]]; then
    printf '[real-mobile] running Android provider path\n'
    start_provider_probe android
    (
        cd "$ROOT/apps/android"
        JAVA_HOME="$JDK_HOME" ./gradlew \
            :app:testDebugUnitTest \
            --rerun-tasks \
            --tests com.nola.llm.provider.OpenAiProviderRealEndToEndTest.streamsConfiguredRealModelWhenExplicitlyEnabled \
            --tests com.nola.llm.provider.OpenAiProviderRealEndToEndTest.realModelUsesValidatedAndroidDataSourceContextAndProducesAuditRecord
    )
    stop_provider_probe

    python3 - "$ANDROID_REPORT" <<'PY'
import sys
import xml.etree.ElementTree as ET

suite = ET.parse(sys.argv[1]).getroot()
report = {
    "tests": int(suite.attrib.get("tests", "0")),
    "skipped": int(suite.attrib.get("skipped", "0")),
    "failures": int(suite.attrib.get("failures", "0")),
    "errors": int(suite.attrib.get("errors", "0")),
}
print(f"[real-mobile] Android report {report}")
if report != {"tests": 2, "skipped": 0, "failures": 0, "errors": 0}:
    raise SystemExit(9)
PY
fi

if [[ "$MOBILE_PLATFORM" == "all" || "$MOBILE_PLATFORM" == "ios" ]]; then
nola_xctest_device_guard_begin "$ROOT" ios_simulator
IOS_DEVICE_ID="${NOLA_IOS_REAL_DEVICE_ID:-}"
if [[ -z "$IOS_DEVICE_ID" ]]; then
    IOS_DEVICE_ID="$(
        xcrun simctl list devices available -j | python3 -c '
import json
import sys

devices = [
    item
    for values in json.load(sys.stdin).get("devices", {}).values()
    for item in values
    if item.get("isAvailable") and "iPhone" in item.get("name", "")
]
booted = next((item for item in devices if item.get("state") == "Booted"), None)
selected = booted or (devices[0] if devices else None)
if not selected:
    raise SystemExit("No available iPhone simulator")
print(selected["udid"])
'
    )"
fi

printf '[real-mobile] preparing iOS provider path on simulator %s\n' "$IOS_DEVICE_ID"
start_provider_probe ios
if [[ "${NOLA_SKIP_IOS_BUILD:-0}" != "1" ]]; then
    xcodebuild build-for-testing \
        -project "$ROOT/apple/Nola.xcodeproj" \
        -scheme Nola \
        -destination "platform=iOS Simulator,id=$IOS_DEVICE_ID" \
        -derivedDataPath "$DERIVED_DATA" \
        CODE_SIGNING_ALLOWED=NO \
        -quiet
fi

XCTESTRUN="$(
    find "$DERIVED_DATA/Build/Products" -maxdepth 1 -name '*.xctestrun' -print -quit
)"
[[ -f "$XCTESTRUN" ]] || {
    printf 'iOS xctestrun is missing under %s\n' "$DERIVED_DATA/Build/Products" >&2
    exit 2
}

python3 - "$XCTESTRUN" "$IOS_DEVICE_ID" <<'PY'
import json
import os
import plistlib
import shutil
import subprocess
import sys
import tempfile

source_path, device_id = sys.argv[1:]
values = {
    "endpoint": os.environ.get("NOLA_REAL_LLM_ENDPOINT", "").strip(),
    "api_key": os.environ.get("NOLA_REAL_LLM_API_KEY", "").strip(),
    "model": os.environ.get("NOLA_REAL_LLM_MODEL", "").strip(),
}
if not all(values.values()):
    raise SystemExit("Nola config is incomplete")

descriptor, run_path = tempfile.mkstemp(
    prefix="Nola_real_",
    suffix=".xctestrun",
    dir=os.path.dirname(source_path),
)
os.close(descriptor)
result_directory = tempfile.mkdtemp(
    prefix="nola-ios-real-result-",
    dir="/tmp",
)
result_path = os.path.join(result_directory, "result.xcresult")

try:
    with open(source_path, "rb") as handle:
        run = plistlib.load(handle)
    target_found = False
    for configuration in run.get("TestConfigurations", []):
        for target in configuration.get("TestTargets", []):
            if target.get("BlueprintName") != "NolaTests":
                continue
            target.setdefault("EnvironmentVariables", {}).update(
                {
                    "NOLA_REAL_LLM_E2E": "1",
                    "NOLA_REAL_LLM_ENDPOINT": values["endpoint"],
                    "NOLA_REAL_LLM_API_KEY": values["api_key"],
                    "NOLA_REAL_LLM_MODEL": values["model"],
                }
            )
            target_found = True
    if not target_found:
        raise SystemExit("NolaTests target is missing from xctestrun")
    with open(run_path, "wb") as handle:
        plistlib.dump(run, handle)

    completed = subprocess.run(
        [
            "xcodebuild",
            "test-without-building",
            "-xctestrun",
            run_path,
            "-destination",
            f"platform=iOS Simulator,id={device_id}",
            "-parallel-testing-enabled",
            "NO",
            "-only-testing:NolaTests/OpenAIProviderTests/testChatCompletionsURL_NormalizesSupportedConfigurationShapes",
            "-only-testing:NolaTests/EndToEndFlowTests/testRealOpenAIProviderStreamsConfiguredModelWhenExplicitlyEnabled",
            "-only-testing:NolaTests/EndToEndFlowTests/testRealModelUsesValidatedDataSourceContextAndRecordsDisclosure",
            "-resultBundlePath",
            result_path,
            "-quiet",
        ],
        check=False,
    )
    report_process = subprocess.run(
        [
            "xcrun",
            "xcresulttool",
            "get",
            "test-results",
            "tests",
            "--path",
            result_path,
            "--compact",
        ],
        check=True,
        stdout=subprocess.PIPE,
        text=True,
    )
    report = json.loads(report_process.stdout)
    summary_process = subprocess.run(
        ["xcrun", "xcresulttool", "get", "test-results", "summary", "--path", result_path, "--compact"],
        check=True,
        stdout=subprocess.PIPE,
        text=True,
    )
    summary = json.loads(summary_process.stdout)
    failures = [
        {
            "test": item.get("testName"),
            "reason": str(item.get("failureText") or item.get("message") or "")[:400],
        }
        for item in summary.get("testFailures", [])
        if isinstance(item, dict)
    ]
    if failures:
        print(f"[real-mobile] iOS failures {json.dumps(failures, ensure_ascii=False)}")
    selected = []

    def collect(value):
        if isinstance(value, dict):
            name = value.get("name")
            if isinstance(name, str) and (
                "testRealOpenAIProvider" in name
                or "testRealModelUsesValidatedDataSource" in name
                or "testChatCompletionsURL" in name
            ):
                selected.append(
                    {
                        "name": name,
                        "status": value.get("result") or value.get("status"),
                        "duration": value.get("durationInSeconds"),
                    }
                )
            for child in value.values():
                collect(child)
        elif isinstance(value, list):
            for child in value:
                collect(child)

    collect(report)
    print(f"[real-mobile] iOS report {json.dumps(selected, ensure_ascii=False)}")
    if completed.returncode != 0 or len(selected) != 3:
        raise SystemExit(completed.returncode or 9)
    if any(item.get("status") != "Passed" for item in selected):
        raise SystemExit(9)
finally:
    try:
        os.unlink(run_path)
    except FileNotFoundError:
        pass
    shutil.rmtree(result_directory, ignore_errors=True)
PY
stop_provider_probe
fi

unset NOLA_REAL_LLM_API_KEY
python3 - "$ROOT" "$MOBILE_PLATFORM" "$ANDROID_REPORT" "$EVIDENCE_WORK_DIR/android-provider.ndjson" "$EVIDENCE_WORK_DIR/ios-provider.ndjson" <<'PY'
from pathlib import Path
import sys

root, platform, android_report, android_trace, ios_trace = sys.argv[1:]
sys.path.insert(0, str(Path(root) / "scripts"))
from real_e2e_evidence import record_step

if platform in {"all", "android"}:
    record_step(
        "android-native-provider-streaming",
        platforms=["android"],
        modules=["provider", "streaming", "data-source", "audit"],
        boundaries=["android-provider", "provider"],
        real_provider_requests=2,
        assertions={"real_provider_stream_passed": True, "no_skipped_tests": True},
        assertion_evidence={
            "real_provider_stream_passed": ["android-provider-traffic"],
            "no_skipped_tests": ["android-test-report"],
        },
        metrics={"androidTests": 2},
        artifacts=[android_report],
        observations=[
            {"id": "android-provider-traffic", "kind": "provider_trace", "path": android_trace},
            {"id": "android-test-report", "kind": "test_report", "path": android_report},
        ],
    )
if platform in {"all", "ios"}:
    record_step(
        "ios-native-provider-streaming",
        platforms=["ios"],
        modules=["provider", "streaming", "data-source", "audit"],
        boundaries=["ios-provider", "provider"],
        real_provider_requests=2,
        assertions={"real_provider_stream_passed": True, "no_skipped_tests": True},
        assertion_evidence={
            "real_provider_stream_passed": ["ios-provider-traffic"],
            "no_skipped_tests": ["ios-provider-traffic"],
        },
        metrics={"iosTests": 3},
        observations=[
            {"id": "ios-provider-traffic", "kind": "provider_trace", "path": ios_trace},
        ],
    )
PY
printf '[real-mobile] passed\n'
