#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Shared real-provider macOS XCTest runner. Product-specific wrappers select
# tests; this file owns secret-safe xctestrun injection and skip detection.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DERIVED_DATA_WAS_CREATED=0
if [[ -n "${NOLA_MAC_REAL_XCTEST_DERIVED_DATA:-${NOLA_MAC_REAL_MEMORY_DERIVED_DATA:-}}" ]]; then
    DERIVED_DATA="${NOLA_MAC_REAL_XCTEST_DERIVED_DATA:-$NOLA_MAC_REAL_MEMORY_DERIVED_DATA}"
    mkdir -p "$DERIVED_DATA"
else
    DERIVED_DATA="$ROOT/.build/real-mac-xctest-derived-data"
    DERIVED_DATA_WAS_CREATED=1
    mkdir -p "$DERIVED_DATA"
fi
CONFIG_PATH="${NOLA_CONFIG_PATH:-$HOME/.nola/config.json}"
ARTIFACTS="$(mktemp -d /tmp/nola-mac-real-memory.XXXXXX)"
printf '[real-mac-xctest] diagnostics: %s\n' "$ARTIFACTS"
RUNTIME_ROOT="$ARTIFACTS/runtime"
RUNTIME_HOME="$RUNTIME_ROOT/home"
RUNTIME_SUPPORT="$RUNTIME_ROOT/ApplicationSupport/Nola"
RUNTIME_CACHE="$RUNTIME_ROOT/Caches/Nola"
RUNTIME_DB="$RUNTIME_SUPPORT/State/nola-v4.db"
UI_SCREENSHOT="$RUNTIME_ROOT/mac-product-ui.png"
UI_ACCESSIBILITY="$RUNTIME_ROOT/mac-product-ui-accessibility.txt"
UI_TIMELINE="$RUNTIME_ROOT/mac-product-ui-timeline.json"
UI_TRANSCRIPT="$RUNTIME_ROOT/mac-product-ui-transcript.json"
AGENT_EVENT_STREAM="$RUNTIME_ROOT/mac-agent-events.json"
PROVIDER_TRACE="$RUNTIME_ROOT/provider.ndjson"
PROVIDER_READY="$RUNTIME_ROOT/provider.endpoint"
PROVIDER_LOG="$RUNTIME_ROOT/provider-probe.log"
SHARED_AGENT_TARGET="${NOLA_MAC_SHARED_AGENT_TARGET:-${NOLA_AGENT_CARGO_TARGET_DIR:-${CARGO_TARGET_DIR:-$ROOT/target}}}"
mkdir -p "$RUNTIME_HOME" "$RUNTIME_SUPPORT/State" "$RUNTIME_CACHE"
mkdir -p "$ARTIFACTS/tmp"
export TMPDIR="$ARTIFACTS/tmp/"
export NOLA_MAC_E2E_HOME="$RUNTIME_HOME"
export NOLA_MAC_E2E_CONFIG_HOME="$RUNTIME_SUPPORT/Configuration"
export NOLA_MAC_E2E_DATA_HOME="$RUNTIME_SUPPORT/Data"
export NOLA_MAC_E2E_CACHE_HOME="$RUNTIME_CACHE"
export NOLA_MAC_E2E_DB_PATH="$RUNTIME_DB"
export NOLA_MAC_E2E_APP_SUPPORT="$RUNTIME_SUPPORT"
export NOLA_MAC_E2E_CACHE_ROOT="$RUNTIME_CACHE"
export NOLA_MAC_REAL_UI_SCREENSHOT="$UI_SCREENSHOT"
export NOLA_MAC_REAL_UI_ACCESSIBILITY="$UI_ACCESSIBILITY"
export NOLA_MAC_REAL_UI_TIMELINE="$UI_TIMELINE"
export NOLA_MAC_REAL_UI_TRANSCRIPT="$UI_TRANSCRIPT"
export NOLA_MAC_E2E_PROVIDER_TRACE="$PROVIDER_TRACE"
export NOLA_MAC_E2E_PROVIDER_READY="$PROVIDER_READY"
export NOLA_MAC_E2E_PROVIDER_LOG="$PROVIDER_LOG"
export NOLA_AGENT_CARGO_TARGET_DIR="$SHARED_AGENT_TARGET"
LOG="$ARTIFACTS/xcodebuild.log"
RESULT="$ARTIFACTS/result.xcresult"
SUCCEEDED=0
ONLY_TESTING="${NOLA_MAC_REAL_ONLY_TESTING:-NolaMacTests/MacAgentCapabilityEndToEndTests/testRealConfiguredModelPersistsAndRecallsMemoryAcrossMacAgentHostConversations}"
EXPECTED_TEST_COUNT="${NOLA_MAC_REAL_EXPECTED_TEST_COUNT:-1}"
EXPECTED_SUCCESSFUL_RUNS="${NOLA_MAC_REAL_MIN_SUCCESSFUL_RUNS:-$EXPECTED_TEST_COUNT}"
EVIDENCE_PROFILE="${NOLA_MAC_REAL_EVIDENCE_PROFILE:-runtime}"
case "$EVIDENCE_PROFILE" in
    runtime|product_ui) ;;
    *)
        printf 'Unsupported NOLA_MAC_REAL_EVIDENCE_PROFILE: %s\n' "$EVIDENCE_PROFILE" >&2
        exit 2
        ;;
esac

cleanup() {
    if [[ "$SUCCEEDED" != "1" ]]; then
        sqlite3 -noheader "$RUNTIME_DB" \
            'SELECT event_json FROM trace_journal_events ORDER BY rowid' \
            >"$AGENT_EVENT_STREAM" 2>/dev/null || : >"$AGENT_EVENT_STREAM"
        if [[ -n "${NOLA_E2E_RECEIPT_PATH:-}" ]]; then
        python3 - "$ROOT" "$RESULT" "$LOG" "$RUNTIME_DB" "$UI_SCREENSHOT" "$UI_ACCESSIBILITY" "$UI_TIMELINE" "$UI_TRANSCRIPT" "$AGENT_EVENT_STREAM" "$PROVIDER_TRACE" <<'PY' || true
import os
from pathlib import Path
import sys

root, result, log, runtime_db, screenshot, accessibility, timeline, transcript, event_stream, provider_trace = sys.argv[1:]
sys.path.insert(0, str(Path(root) / "scripts"))
from real_e2e_evidence import record_step

paths = {
    "ui_screenshot": Path(screenshot),
    "accessibility_tree": Path(accessibility),
    "ui_timeline": Path(timeline),
    "transcript": Path(transcript),
    "event_stream": Path(event_stream),
    "provider_trace": Path(provider_trace),
    "persistence_store": Path(runtime_db),
}
observations = [
    {"id": f"mac-failed-{kind}", "kind": kind, "path": path}
    for kind, path in paths.items() if path.is_file()
]
record_step(
    os.environ.get("NOLA_MAC_REAL_EVIDENCE_STEP", "mac-real-xctest-failed"),
    platforms=["mac"],
    modules=[item for item in os.environ.get("NOLA_MAC_REAL_EVIDENCE_MODULES", "").split(",") if item],
    boundaries=[item for item in os.environ.get("NOLA_MAC_REAL_EVIDENCE_BOUNDARIES", "").split(",") if item],
    assertions={"selected_real_tests_executed": False, "selected_real_tests_passed": False},
    observations=observations,
    artifacts=[path for path in [Path(result), Path(log), Path(runtime_db), Path(transcript)] if path.exists()],
)
PY
        fi
    fi
    if [[ "$SUCCEEDED" == "1" && "${NOLA_MAC_REAL_MEMORY_PRESERVE_ARTIFACTS:-0}" != "1" && "${NOLA_MAC_REAL_MEASUREMENT:-0}" != "1" ]]; then
        rm -rf "$ARTIFACTS"
    else
        printf '[real-mac-xctest] diagnostics preserved at %s\n' "$ARTIFACTS" >&2
    fi
    if [[ "$DERIVED_DATA_WAS_CREATED" == "1" && "${NOLA_MAC_REAL_PRESERVE_DERIVED_DATA:-0}" != "1" ]]; then
        rm -rf "$DERIVED_DATA"
    fi
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

[[ -f "$CONFIG_PATH" ]] || {
    printf 'Missing Nola config: %s\n' "$CONFIG_PATH" >&2
    exit 2
}

if [[ "${NOLA_SKIP_MAC_MEMORY_BUILD:-0}" != "1" ]]; then
    printf '[real-mac-xctest] building macOS tests\n'
    xcodebuild build-for-testing \
        -project "$ROOT/Nola.xcodeproj" \
        -scheme NolaMac \
        -configuration Test \
        -destination 'platform=macOS,arch=arm64' \
        -destination-timeout 10 \
        -derivedDataPath "$DERIVED_DATA" \
        ARCHS=arm64 \
        ONLY_ACTIVE_ARCH=YES \
        CODE_SIGNING_ALLOWED=NO \
        SWIFT_EMIT_LOC_STRINGS=NO \
        -quiet >"$LOG" 2>&1 || {
            printf '[real-mac-xctest] build failed\n' >&2
            rg -n -C 4 '(^|[^A-Za-z])error:|BUILD FAILED|Command .* failed|PhaseScriptExecution' \
                "$LOG" 2>/dev/null | tail -n 120 >&2 || true
            exit 65
        }
fi

XCTESTRUN="$(python3 - "$DERIVED_DATA/Build/Products" <<'PY'
import os
import plistlib
import sys

products = sys.argv[1]
matches = []
for name in os.listdir(products) if os.path.isdir(products) else []:
    if not name.endswith(".xctestrun"):
        continue
    path = os.path.join(products, name)
    try:
        with open(path, "rb") as handle:
            run = plistlib.load(handle)
    except (OSError, plistlib.InvalidFileException):
        continue
    contains_mac_tests = any(
        target.get("BlueprintName") == "NolaMacTests"
        for configuration in run.get("TestConfigurations", [])
        for target in configuration.get("TestTargets", [])
    )
    if contains_mac_tests:
        matches.append((os.path.getmtime(path), path))

if matches:
    print(max(matches)[1])
PY
)"
[[ -f "$XCTESTRUN" ]] || {
    printf 'macOS xctestrun containing NolaMacTests is missing under %s\n' "$DERIVED_DATA/Build/Products" >&2
    exit 2
}

printf '[real-mac-xctest] running Swift -> Agent Host -> Core -> real LLM\n'
python3 - "$CONFIG_PATH" "$XCTESTRUN" "$RESULT" "$LOG" "$ONLY_TESTING" "$EXPECTED_TEST_COUNT" "$ROOT" <<'PY'
import json
import os
import plistlib
import shutil
import subprocess
import sys
import tempfile
import signal
from pathlib import Path

config_path, source_path, result_path, log_path, only_testing, expected_test_count, root = sys.argv[1:]
sys.path.insert(0, str(Path(root) / "scripts"))
from mac_measurement_environment import configure as configure_measurement
measurement_mode = os.environ.get("NOLA_MAC_REAL_MEASUREMENT", "0")
if measurement_mode not in ("0", "1"):
    raise SystemExit("NOLA_MAC_REAL_MEASUREMENT must be 0 or 1")
for key in ("NOLA_MEASUREMENT_FILE", "NOLA_MEASUREMENT_BUILD_DIGEST"):
    os.environ.pop(key, None)
with open(config_path, encoding="utf-8") as handle:
    config = json.load(handle)
    llm = config.get("llm", {})
selection = llm.get("default_model")
connections = llm.get("connections")
if not isinstance(selection, dict) or not isinstance(connections, list):
    raise SystemExit("Nola config must define llm.connections and llm.default_model")
selected = [item for item in connections if isinstance(item, dict) and item.get("id") == selection.get("connection")]
if len(selected) != 1:
    raise SystemExit("Default model must reference exactly one configured connection")
connection = selected[0]
models = connection.get("models", [])
model_id = selection.get("model")
if not isinstance(model_id, str) or not any(isinstance(item, dict) and item.get("id") == model_id for item in models):
    raise SystemExit("Default model is absent from its connection")
key_variable = connection.get("api_key_env")
if not isinstance(key_variable, str) or not key_variable:
    raise SystemExit("Connection must name api_key_env")
values = {"endpoint": connection.get("endpoint"), "api_key": os.environ.get(key_variable), "model": model_id}
if not all(isinstance(value, str) and value.strip() for value in values.values()):
    raise SystemExit("Configured endpoint, model or credential environment is unavailable")
wire_protocol = connection.get("protocol")
if wire_protocol not in ("chat_completions", "responses", "anthropic"):
    raise SystemExit("Unsupported configured real-test wire protocol")
if os.environ.get("NOLA_MAC_REAL_WIRE_PROTOCOL", wire_protocol) != wire_protocol:
    raise SystemExit("Real-test protocol contradicts the configured connection")

model_fixture_mode = os.environ.get("NOLA_MAC_REAL_REQUIRE_MODEL_FIXTURE", "0")
if model_fixture_mode not in ("0", "1"):
    raise SystemExit("NOLA_MAC_REAL_REQUIRE_MODEL_FIXTURE must be 0 or 1")
model_fixture_json = None
if model_fixture_mode == "1":
    from e2e.model_execution_fixture import fixture
    declared_environment = dict(os.environ)
    declared_environment["NOLA_REAL_E2E_MODEL_PROTOCOL"] = wire_protocol
    declared = fixture(config, declared_environment, model=values["model"], endpoint=values["endpoint"], require_tool=False)
    if ("NOLA_MAC_REAL_WIRE_PROTOCOL" in os.environ
            and declared["wireProtocol"] != wire_protocol):
        raise SystemExit("Real XCTest protocol contradicts the declared model fixture")
    wire_protocol = declared["wireProtocol"]
    model_fixture_json = json.dumps(declared, separators=(",", ":"), sort_keys=True)


probe_log = open(os.environ["NOLA_MAC_E2E_PROVIDER_LOG"], "wb")
interrupt_bytes = int(os.environ.get("NOLA_MAC_REAL_INTERRUPT_AFTER_TEXT_BYTES", "0"))
truncate_tokens = int(os.environ.get("NOLA_MAC_REAL_COMPACTION_TRUNCATE_TOKENS", "0"))
if truncate_tokens < 0:
    raise SystemExit("Truncation threshold must be non-negative")
if interrupt_bytes < 0:
    raise SystemExit("Interruption threshold must be non-negative")
probe = subprocess.Popen(
    [
        sys.executable,
        str(Path(root) / "scripts/e2e/provider_probe.py"),
        "--upstream", values["endpoint"],
        "--log", os.environ["NOLA_MAC_E2E_PROVIDER_TRACE"],
        "--ready", os.environ["NOLA_MAC_E2E_PROVIDER_READY"],
        "--interrupt-once-after-text-bytes", str(interrupt_bytes),
        "--truncate-compaction-once-tokens", str(truncate_tokens),
        # Let the product's HTTP/stream deadlines decide slow-model behavior.
        # The evidence relay must not cut off cold prefill after its 45s default.
        "--upstream-timeout-seconds", os.environ.get("NOLA_MAC_REAL_PROXY_TIMEOUT_SECONDS", "240"),
    ] + (["--capture-last-user-message"] if os.environ.get("NOLA_MAC_REAL_CAPTURE_TEST_CONTEXT") == "1" else []),
    stdout=probe_log,
    stderr=subprocess.STDOUT,
)
import time
probe_ready_deadline = time.monotonic() + float(os.environ.get("NOLA_MAC_PROVIDER_PROBE_STARTUP_SECONDS", "30"))
while time.monotonic() < probe_ready_deadline:
    ready = Path(os.environ["NOLA_MAC_E2E_PROVIDER_READY"])
    if ready.is_file() and ready.stat().st_size:
        # Direct mode validates endpoint-derived protocol behavior. It has no
        # proxy wire evidence; do not report an empty probe as a captured run.
        if os.environ.get("NOLA_MAC_REAL_DIRECT_PROVIDER") != "1":
            values["endpoint"] = json.loads(ready.read_text())["endpoint"]
        break
    if probe.poll() is not None:
        raise SystemExit("Mac provider evidence probe exited before becoming ready")
    time.sleep(0.05)
else:
    probe.terminate()
    try:
        probe.wait(timeout=3)
    except subprocess.TimeoutExpired:
        probe.kill()
        probe.wait(timeout=3)
    raise SystemExit("Mac provider evidence probe did not become ready")

descriptor, run_path = tempfile.mkstemp(
    prefix="NolaMac_real_memory_",
    suffix=".xctestrun",
    dir=os.path.dirname(source_path),
)
os.close(descriptor)
try:
    with open(source_path, "rb") as handle:
        run = plistlib.load(handle)
    target_found = False
    for configuration in run.get("TestConfigurations", []):
        for target in configuration.get("TestTargets", []):
            if target.get("BlueprintName") != "NolaMacTests":
                continue
            if target_found:
                raise SystemExit("Real Mac runner requires exactly one NolaMacTests target")
            target.setdefault("EnvironmentVariables", {}).update(
                {
                    "NOLA_TEST_MODE": "1",
                    "NOLA_REQUIRE_MAC_SYSTEM_PERMISSIONS": "0",
                    "NOLA_USE_REAL_KEYCHAIN_IN_TESTS": "0",
                    "NOLA_REAL_LLM_E2E": "1",
                    "NOLA_REAL_LLM_ENDPOINT": values["endpoint"],
                    "NOLA_REAL_LLM_API_KEY": values["api_key"],
                    "NOLA_REAL_LLM_MODEL": values["model"],
                    "NOLA_REAL_LLM_WIRE_PROTOCOL": wire_protocol,
                    "NOLA_LLM_MAX_RETRIES": os.environ.get("NOLA_LLM_MAX_RETRIES", "0"),
                    "NOLA_REAL_COMPACTION_REASONING_NONE": "1" if truncate_tokens else "0",
                    "NOLA_REAL_COMPACTION_WINDOW": os.environ.get("NOLA_REAL_COMPACTION_WINDOW", "16000"),
                    # Separate an early maintenance trigger from the hard
                    # request window; never shrink the latter just to trigger
                    # a summary on a small deterministic fixture.
                    "NOLA_AGENT_CONTEXT_TOKEN_THRESHOLD": os.environ.get("NOLA_AGENT_CONTEXT_TOKEN_THRESHOLD", ""),
                    "NOLA_MAC_STREAM_TEST_DEADLINE_SECONDS": "480" if interrupt_bytes else "240",
                    "HOME": os.environ["NOLA_MAC_E2E_HOME"],
                    "XDG_CONFIG_HOME": os.environ["NOLA_MAC_E2E_CONFIG_HOME"],
                    "XDG_DATA_HOME": os.environ["NOLA_MAC_E2E_DATA_HOME"],
                    "XDG_CACHE_HOME": os.environ["NOLA_MAC_E2E_CACHE_HOME"],
                    "NOLA_DB_PATH": os.environ["NOLA_MAC_E2E_DB_PATH"],
                    "NOLA_MAC_APP_SUPPORT": os.environ["NOLA_MAC_E2E_APP_SUPPORT"],
                    "NOLA_MAC_CACHE_ROOT": os.environ["NOLA_MAC_E2E_CACHE_ROOT"],
                    "NOLA_NATIVE_BROWSER_PORT": "0",
                    "NOLA_MAC_REAL_UI_SCREENSHOT": os.environ["NOLA_MAC_REAL_UI_SCREENSHOT"],
                    "NOLA_MAC_REAL_UI_ACCESSIBILITY": os.environ["NOLA_MAC_REAL_UI_ACCESSIBILITY"],
                    "NOLA_MAC_REAL_UI_TIMELINE": os.environ["NOLA_MAC_REAL_UI_TIMELINE"],
                    "NOLA_MAC_REAL_UI_TRANSCRIPT": os.environ["NOLA_MAC_REAL_UI_TRANSCRIPT"],
                    "NOLA_CLI_FORCE_TOOL_SET": "all",
                }
            )
            if model_fixture_json is not None:
                target["EnvironmentVariables"]["NOLA_REAL_LLM_MODEL_FIXTURE_JSON"] = model_fixture_json
            configure_measurement(target, source_path, Path(result_path).parent,
                                  measurement_mode == "1")
            target_found = True
    if not target_found:
        raise SystemExit("NolaMacTests target is missing from xctestrun")
    with open(run_path, "wb") as handle:
        plistlib.dump(run, handle)

    if os.path.exists(result_path):
        shutil.rmtree(result_path)
    with open(log_path, "ab") as log:
        command = [
                "xcodebuild",
                "test-without-building",
                "-xctestrun",
                run_path,
                "-destination",
                "platform=macOS,arch=arm64",
                "-destination-timeout",
                "10",
                "-parallel-testing-enabled",
                "NO",
        ]
        for identifier in only_testing.split("|||"):
            identifier = identifier.strip()
            if identifier:
                command.append(f"-only-testing:{identifier}")
        command.extend(["-resultBundlePath", result_path, "-quiet"])
        test_timeout = int(os.environ.get("NOLA_MAC_REAL_XCTEST_TIMEOUT_SECONDS", "600"))
        test_process = subprocess.Popen(
            command,
            stdout=log,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )
        try:
            returncode = test_process.wait(timeout=test_timeout)
        except subprocess.TimeoutExpired:
            os.killpg(test_process.pid, signal.SIGTERM)
            try:
                test_process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                os.killpg(test_process.pid, signal.SIGKILL)
                test_process.wait()
            raise SystemExit(
                f"Real macOS XCTest exceeded {test_timeout}s; process group terminated"
            )
        except KeyboardInterrupt:
            os.killpg(test_process.pid, signal.SIGTERM)
            try:
                test_process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                os.killpg(test_process.pid, signal.SIGKILL)
                test_process.wait()
            raise
    if returncode != 0:
        raise SystemExit(returncode)

    summary_process = subprocess.run(
        [
            "xcrun",
            "xcresulttool",
            "get",
            "test-results",
            "summary",
            "--path",
            result_path,
        ],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
        text=True,
        timeout=60,
    )
    if summary_process.returncode != 0:
        raise SystemExit("Unable to read xcresult summary")
    summary = json.loads(summary_process.stdout)
    Path(result_path).with_suffix(".summary.json").write_text(
        summary_process.stdout, encoding="utf-8"
    )
    total = int(summary.get("totalTestCount", 0))
    passed = int(summary.get("passedTests", 0))
    skipped = int(summary.get("skippedTests", 0))
    failed = int(summary.get("failedTests", 0))
    print(
        json.dumps(
            {"total": total, "passed": passed, "skipped": skipped, "failed": failed},
            separators=(",", ":"),
        )
    )
    expected = int(expected_test_count)
    if total != expected or passed != expected or skipped != 0 or failed != 0:
        raise SystemExit(
            f"Real macOS tests did not execute exactly {expected} time(s) and pass"
        )
    if truncate_tokens:
        from compaction_continuation_evidence import verify as verify_compaction_continuation
        responses = [json.loads(line) for line in Path(os.environ["NOLA_MAC_E2E_PROVIDER_TRACE"]).read_text().splitlines() if line.strip()]
        truncated = [item for item in responses if item.get("injectedCompactionTruncation")]
        if (len(truncated) != 1 or "max_output_tokens" not in truncated[0].get("finishReasons", [])
                or not truncated[0].get("responseTextBytes")):
            raise SystemExit("Compaction acceptance did not trigger a real upstream truncation with partial text")
        verify_compaction_continuation(os.environ["NOLA_MAC_E2E_DB_PATH"])
    if interrupt_bytes:
        responses = [json.loads(line) for line in Path(os.environ["NOLA_MAC_E2E_PROVIDER_TRACE"]).read_text().splitlines() if line.strip()]
        interrupted = [item for item in responses if item.get("injectedInterruption")]
        if len(interrupted) != 1 or interrupted[0].get("responseTextBytes", 0) < interrupt_bytes:
            raise SystemExit("Recovery acceptance did not actually interrupt exactly one partial answer")
finally:
    probe.terminate()
    try:
        probe.wait(timeout=5)
    except subprocess.TimeoutExpired:
        probe.kill()
    probe_log.close()
    try:
        os.remove(run_path)
    except FileNotFoundError:
        pass
PY

# A green XCTest result is not sufficient evidence for chat correctness.
# Compare production-validated exact terminals with persisted immutable seals.
python3 - "$RUNTIME_DB" "$UI_TIMELINE" "$EXPECTED_SUCCESSFUL_RUNS" "$UI_SCREENSHOT" "$UI_ACCESSIBILITY" "$UI_TRANSCRIPT" "$EVIDENCE_PROFILE" "$ROOT" <<'PY'
import json
import sqlite3
import sys
from pathlib import Path

database_path, timeline_path, expected_count, screenshot_path, accessibility_path, transcript_path, evidence_profile, root = sys.argv[1:]
sys.path.insert(0, str(Path(root) / "scripts"))
from verify_mac_ui_terminal import verify as verify_terminal
if evidence_profile != "product_ui":
    print(json.dumps({"evidenceProfile": evidence_profile}, separators=(",", ":")))
    raise SystemExit(0)

with sqlite3.connect(Path(database_path).resolve().as_uri() + "?mode=ro", uri=True) as connection:
    terminal_summary = verify_terminal(connection,
        json.loads(Path(transcript_path).read_text(encoding="utf-8")), int(expected_count))

problems = []
required_artifacts = {
    "UI screenshot": (Path(screenshot_path), 10_000),
    "accessibility tree": (Path(accessibility_path), 64),
    "UI timeline": (Path(timeline_path), 16),
    "UI transcript": (Path(transcript_path), 32),
}
for label, (path, minimum_size) in required_artifacts.items():
    if not path.is_file() or path.stat().st_size < minimum_size:
        problems.append(f"{label} is missing or incomplete: {path}")

timeline_file = Path(timeline_path)
if timeline_file.is_file():
    timeline = json.loads(timeline_file.read_text(encoding="utf-8"))
    if not timeline:
        problems.append("UI timeline is empty")
    previous = {}
    for sample in timeline:
        stage = sample.get("stage")
        count = int(sample.get("assistantCharacters", 0))
        if stage in previous and count < previous[stage]:
            problems.append(
                f"assistant text regressed in {stage}: {previous[stage]} -> {count}"
            )
            break
        previous[stage] = count
    if timeline and timeline[-1].get("running"):
        problems.append("UI timeline ended while the run was still active")

if problems:
    raise SystemExit("Real Mac state validation failed: " + "; ".join(problems))

print(json.dumps(terminal_summary, separators=(",", ":")))
PY

sqlite3 -noheader "$RUNTIME_DB" \
    'SELECT event_json FROM trace_journal_events ORDER BY rowid' \
    >"$AGENT_EVENT_STREAM" 2>/dev/null || : >"$AGENT_EVENT_STREAM"
python3 - "$ROOT" "$RESULT" "$LOG" "$EXPECTED_TEST_COUNT" "$RUNTIME_DB" "$UI_SCREENSHOT" "$UI_ACCESSIBILITY" "$UI_TIMELINE" "$UI_TRANSCRIPT" "$AGENT_EVENT_STREAM" "$PROVIDER_TRACE" "$EVIDENCE_PROFILE" <<'PY'
import json
import os
from pathlib import Path
import sys

root, result, log, expected_count, runtime_db, screenshot, accessibility, timeline, transcript, event_stream, provider_trace, evidence_profile = sys.argv[1:]
sys.path.insert(0, str(Path(root) / "scripts"))
from real_e2e_evidence import record_step
from provider_trace_metrics import provider_trace_metrics

# Legacy Trace is optional diagnostic output, not a second terminal authority.
# Canonical results were checked against production-validated receipts above.

provider_path = Path(provider_trace)
if not provider_path.is_file() or provider_path.stat().st_size == 0:
    raise SystemExit("real provider trace is missing or empty")
provider_metrics = provider_trace_metrics([provider_path])
summary_path = Path(result).with_suffix(".summary.json")
summary = json.loads(summary_path.read_text(encoding="utf-8"))
expected = int(expected_count)
test_assertions = {
    "selected_real_tests_executed": summary.get("totalTestCount") == expected,
    "selected_real_tests_passed": summary.get("passedTests") == expected
        and summary.get("failedTests") == 0,
    "no_selected_test_skipped": summary.get("skippedTests") == 0,
}
if expected < 1 or not all(test_assertions.values()):
    raise SystemExit("XCTest summary does not prove all selected tests passed without skips")

text_stream_gate = os.environ.get("NOLA_MAC_REAL_REQUIRE_TEXT_STREAM", "0")
budget_gate = os.environ.get("NOLA_MAC_REAL_REQUIRE_CONTEXT_BUDGET", "0")
if budget_gate not in ("0", "1"):
    raise SystemExit("NOLA_MAC_REAL_REQUIRE_CONTEXT_BUDGET must be 0 or 1")
if budget_gate == "1":
    from provider_budget_acceptance import require_context_budget
    try:
        budget_metrics = require_context_budget([
            json.loads(line) for line in provider_path.read_text().splitlines() if line.strip()
        ], int(os.environ["NOLA_REAL_COMPACTION_WINDOW"]))
    except (ValueError, OSError, KeyError) as error:
        raise SystemExit(f"Real Mac context-budget acceptance failed: {error}")
    provider_metrics["contextBudget"] = budget_metrics
    print(json.dumps({"contextBudget": budget_metrics}))
if text_stream_gate not in ("0", "1"):
    raise SystemExit("NOLA_MAC_REAL_REQUIRE_TEXT_STREAM must be 0 or 1")
if text_stream_gate == "1":
    from provider_stream_acceptance import require_text_stream
    try:
        count = require_text_stream([
            json.loads(line) for line in provider_path.read_text().splitlines() if line.strip()
        ])
    except (ValueError, OSError) as error:
        raise SystemExit(f"Real Mac text-stream acceptance failed: {error}")
    print(json.dumps({"observedTextStreamResponses": count}))

modules = [item for item in os.environ.get("NOLA_MAC_REAL_EVIDENCE_MODULES", "").split(",") if item]
boundaries = [item for item in os.environ.get("NOLA_MAC_REAL_EVIDENCE_BOUNDARIES", "").split(",") if item]
step = os.environ.get("NOLA_MAC_REAL_EVIDENCE_STEP", "mac-real-xctest")
observation_paths = {
    "ui_screenshot": Path(screenshot),
    "accessibility_tree": Path(accessibility),
    "ui_timeline": Path(timeline),
    "transcript": Path(transcript),
    "event_stream": Path(event_stream),
    "provider_trace": Path(provider_trace),
    "persistence_store": Path(runtime_db),
    "xctest_summary": summary_path,
}
observations = [
    {"id": f"mac-{kind}", "kind": kind, "path": path}
    for kind, path in observation_paths.items() if path.is_file()
]
assertions = {
    **test_assertions,
    "provider_trace_recorded": True,
}
assertion_evidence = {
    "selected_real_tests_executed": ["mac-xctest_summary"],
    "selected_real_tests_passed": ["mac-xctest_summary"],
    "no_selected_test_skipped": ["mac-xctest_summary"],
    "provider_trace_recorded": ["mac-provider_trace"],
}
if budget_gate == "1":
    assertions["actual_requests_within_context_budget"] = True
    assertion_evidence["actual_requests_within_context_budget"] = ["mac-provider_trace"]
if evidence_profile == "product_ui":
    assertions.update({
        "isolated_database": Path(runtime_db).is_file(),
        "ui_artifacts_recorded": Path(timeline).is_file() and Path(screenshot).is_file(),
        "run_succeeded": True,
    })
    assertion_evidence.update({
        "isolated_database": ["mac-persistence_store"],
        "ui_artifacts_recorded": ["mac-ui_timeline", "mac-ui_screenshot"],
        "run_succeeded": ["mac-transcript", "mac-persistence_store"],
    })
record_step(
    step,
    platforms=["mac"],
    modules=modules,
    boundaries=boundaries,
    real_provider_requests=provider_metrics["successfulModelRequests"],
    assertions=assertions,
    assertion_evidence=assertion_evidence,
    metrics={"selectedTestCount": expected, "providerTrace": provider_metrics},
    observations=observations,
    artifacts=[path for path in map(Path, [result, log, runtime_db, transcript]) if path.exists()],
)
PY

SUCCEEDED=1
printf '[real-mac-xctest] passed\n'
