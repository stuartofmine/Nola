#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Real iOS/Android production-provider acceptance against an isolated Nola Web Agent Host.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
. "$ROOT/scripts/lib/xcode_test_device_guard.sh"
REAL_USER_HOME="$HOME"
CONFIG_PATH="${NOLA_CONFIG_PATH:-$HOME/.nola/config.json}"
if [[ -n "${NOLA_REAL_CLI_BINARY:-}" ]]; then
    BINARY="$NOLA_REAL_CLI_BINARY"
else
    CARGO_TARGET_ROOT="$(
        cargo metadata --manifest-path "$ROOT/apps/cli/Cargo.toml" --no-deps --format-version 1 \
            | python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])'
    )"
    BINARY="$CARGO_TARGET_ROOT/release/nola"
fi
JDK_HOME="${NOLA_ANDROID_JAVA_HOME:-/Library/Java/JavaVirtualMachines/temurin-21.jdk/Contents/Home}"
DERIVED_DATA_WAS_CREATED=0
if [[ -n "${NOLA_IOS_REAL_DERIVED_DATA:-}" ]]; then
    DERIVED_DATA="$NOLA_IOS_REAL_DERIVED_DATA"
    mkdir -p "$DERIVED_DATA"
elif [[ "${NOLA_IOS_REAL_CLEAN_BUILD:-0}" == "1" ]]; then
    DERIVED_DATA="$(mktemp -d "${TMPDIR:-/tmp}/nola-ios-real-host-derived-data.XXXXXX")"
    DERIVED_DATA_WAS_CREATED=1
else
    DERIVED_DATA="${NOLA_REAL_E2E_CACHE_ROOT:-$ROOT/.artifacts/real-e2e-cache}/ios-derived-data"
    mkdir -p "$DERIVED_DATA"
fi
RUNTIME_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/nola-real-mobile-host.XXXXXX")"
RUNTIME_HOME="$RUNTIME_ROOT/home"
RUNTIME_PROJECT="$RUNTIME_ROOT/project"
mkdir -p "$RUNTIME_PROJECT"
SERVER_LOG="$RUNTIME_ROOT/web-host.log"
DESTRUCTIVE_FILE="$RUNTIME_ROOT/permission-deny.txt"
SERVER_PID=""
PROVIDER_PROBE_PID=""
TOKEN_PATH=""
PROVIDER_TRACE="$RUNTIME_ROOT/provider.ndjson"
PROVIDER_READY="$RUNTIME_ROOT/provider.endpoint"
IOS_UI_TIMELINE="$RUNTIME_ROOT/ios-ui-timeline.json"
IOS_UI_SCREENSHOT="$RUNTIME_ROOT/ios-ui.png"
IOS_UI_ACCESSIBILITY="$RUNTIME_ROOT/ios-ui-accessibility.txt"
IOS_UI_TRANSCRIPT="$RUNTIME_ROOT/ios-ui-transcript.json"
IOS_UI_EVENTS="$RUNTIME_ROOT/ios-host-events.json"
IOS_UI_PERSISTENCE_AUDIT="$RUNTIME_ROOT/ios-persistence-audit.json"
IOS_TEST_STATUS_PATH="$RUNTIME_ROOT/ios-test-status.txt"
IOS_TEST_OUTPUT="$RUNTIME_ROOT/ios-xctest-output.log"

cleanup() {
    local status=$?
    if [[ -n "$SERVER_PID" ]]; then
        kill "$SERVER_PID" >/dev/null 2>&1 || true
        wait "$SERVER_PID" >/dev/null 2>&1 || true
    fi
    if [[ -n "$PROVIDER_PROBE_PID" ]]; then
        kill "$PROVIDER_PROBE_PID" >/dev/null 2>&1 || true
        wait "$PROVIDER_PROBE_PID" >/dev/null 2>&1 || true
    fi
    if [[ "$status" -ne 0 && -f "$SERVER_LOG" ]]; then
        # The Host may include configured provider details in diagnostics.
        # Preserve the private artifact without echoing it into the test log.
        printf '[real-mobile-host] isolated Host diagnostics: %s\n' "$SERVER_LOG" >&2
    fi
    if [[ -f "$RUNTIME_HOME/.nola/config.json" ]]; then
        printf '{}\n' > "$RUNTIME_HOME/.nola/config.json"
    fi
    rm -f "$DESTRUCTIVE_FILE"
    if [[ "$status" -ne 0 || "${NOLA_REAL_MOBILE_PRESERVE_RUNTIME:-0}" == "1" ]]; then
        [[ -z "$TOKEN_PATH" ]] || rm -f "$TOKEN_PATH"
        printf '[real-mobile-host] preserved isolated runtime: %s\n' "$RUNTIME_ROOT" >&2
    else
        rm -rf "$RUNTIME_ROOT"
    fi
    unset NOLA_REAL_WEB_AGENT_HOST_TOKEN
    if [[ "$DERIVED_DATA_WAS_CREATED" == "1" && "${NOLA_IOS_REAL_PRESERVE_DERIVED_DATA:-0}" != "1" ]]; then
        rm -rf "$DERIVED_DATA"
    fi
    nola_xctest_device_guard_end "$ROOT" ios_simulator
    return "$status"
}
trap cleanup EXIT

[[ -f "$CONFIG_PATH" ]] || { printf 'Missing Nola config: %s\n' "$CONFIG_PATH" >&2; exit 2; }
[[ -x "$JDK_HOME/bin/java" ]] || { printf 'JDK 21 is missing: %s\n' "$JDK_HOME" >&2; exit 2; }
if [[ "${NOLA_SKIP_ANDROID:-0}" == "1" && "${NOLA_SKIP_IOS:-0}" == "1" ]]; then
    printf 'Android and iOS cannot both be skipped in the real mobile Host acceptance.\n' >&2
    exit 2
fi

if [[ "${NOLA_SKIP_CLI_BUILD:-0}" != "1" ]]; then
    cargo build --release --bin nola --manifest-path "$ROOT/apps/cli/Cargo.toml"
fi
[[ -x "$BINARY" ]] || { printf 'Nola CLI executable is missing: %s\n' "$BINARY" >&2; exit 2; }

mkdir -p "$RUNTIME_HOME/.nola"
# Keep Nola's database, configuration, sessions, and generated files isolated,
# while exercising the user's genuinely installed and authenticated execution
# backends. Availability probes and the spawned CLIs both resolve credentials
# from HOME, so omitting these mappings makes an installed Codex/Claude/OpenCode
# look unavailable and turns the acceptance test into a false negative.
for name in .codex .claude .opencode; do
    if [[ -e "$REAL_USER_HOME/$name" && ! -e "$RUNTIME_HOME/$name" ]]; then
        ln -s "$REAL_USER_HOME/$name" "$RUNTIME_HOME/$name"
    fi
done
if [[ -e "$REAL_USER_HOME/.claude.json" && ! -e "$RUNTIME_HOME/.claude.json" ]]; then
    ln -s "$REAL_USER_HOME/.claude.json" "$RUNTIME_HOME/.claude.json"
fi
mkdir -p "$RUNTIME_HOME/.config"
if [[ -e "$REAL_USER_HOME/.config/opencode" && ! -e "$RUNTIME_HOME/.config/opencode" ]]; then
    ln -s "$REAL_USER_HOME/.config/opencode" "$RUNTIME_HOME/.config/opencode"
fi
printf 'KEEP_UNTIL_PERMISSION_DENIED\n' > "$DESTRUCTIVE_FILE"
IOS_UI_MARKER="IOS-RENDERED-TOOL-$(uuidgen | tr '[:lower:]' '[:upper:]')"
IOS_UI_SOURCE="$RUNTIME_ROOT/ios-rendered-source.md"
printf '# Source\n\nMarker: %s\n' "$IOS_UI_MARKER" > "$IOS_UI_SOURCE"
# Resolve only the current canonical model connection. Never print credential
# values or their environment variable names into the acceptance log.
LLM_ENDPOINT="$(python3 - "$CONFIG_PATH" "$RUNTIME_HOME/.nola/config.json" <<'PYCONFIG'
import json
import os
import sys

llm = json.load(open(sys.argv[1], encoding="utf-8")).get("llm", {})
selected = llm.get("default_model")
if not isinstance(selected, dict) or set(selected) != {"connection", "model"}:
    raise SystemExit("A canonical llm.default_model selection is required")
connections = [item for item in llm.get("connections", []) if item.get("id") == selected["connection"]]
if len(connections) != 1:
    raise SystemExit("The selected canonical connection is not unique")
connection = connections[0]
models = [item for item in connection.get("models", []) if item.get("id") == selected["model"]]
if len(models) != 1 or not str(connection.get("endpoint", "")).strip():
    raise SystemExit("The selected canonical model or endpoint is unavailable")
credential_env = connection.get("api_key_env")
if not isinstance(credential_env, str) or not os.environ.get(credential_env, "").strip():
    raise SystemExit("The selected connection credential environment is unavailable")
# Keep only this deliberately selected connection in the disposable Host.
payload = {"llm": {"connections": [connection], "default_model": selected}, "network_policy": "allow"}
if os.environ.get("NOLA_REAL_MOBILE_COMPACTION_E2E") == "1":
    payload["runtime"] = {"compaction": {"context_window_tokens": 16000, "token_threshold": 12000}}
with open(sys.argv[2], "w", encoding="utf-8") as target:
    json.dump(payload, target)
print(connection["endpoint"])
PYCONFIG
)"
python3 "$ROOT/scripts/e2e/provider_probe.py" \
    --upstream "$LLM_ENDPOINT" \
    --log "$PROVIDER_TRACE" \
    --ready "$PROVIDER_READY" \
    >"$RUNTIME_ROOT/provider-probe.log" 2>&1 &
PROVIDER_PROBE_PID=$!
for _ in $(seq 1 100); do
    [[ -s "$PROVIDER_READY" ]] && break
    kill -0 "$PROVIDER_PROBE_PID" 2>/dev/null || break
    sleep 0.05
done
[[ -s "$PROVIDER_READY" ]] || {
    cat "$RUNTIME_ROOT/provider-probe.log" >&2 || true
    exit 2
}
PROVIDER_ENDPOINT="$(python3 - "$PROVIDER_READY" <<'PY'
import json
import sys
print(json.load(open(sys.argv[1], encoding="utf-8"))["endpoint"])
PY
)"
python3 - "$RUNTIME_HOME/.nola/config.json" "$PROVIDER_ENDPOINT" <<'PYCONFIG'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as source:
    payload = json.load(source)
payload["llm"]["connections"][0]["endpoint"] = sys.argv[2]
with open(sys.argv[1], "w", encoding="utf-8") as target:
    json.dump(payload, target)
PYCONFIG
# Import exactly the same current model catalog into the isolated product Store.
(
    cd "$RUNTIME_PROJECT"
    HOME="$RUNTIME_HOME" NOLA_DB_PATH="$RUNTIME_ROOT/nola.sqlite" \
        "$BINARY" model apply-config "$RUNTIME_HOME/.nola/config.json"
) >"$RUNTIME_ROOT/model-import.log" 2>&1

PORT="$(python3 - <<'PY'
import socket
with socket.socket() as listener:
    listener.bind(("127.0.0.1", 0))
    print(listener.getsockname()[1])
PY
)"
HOST_IP="${NOLA_REAL_WEB_AGENT_HOST_IP:-$(ipconfig getifaddr en0 2>/dev/null || true)}"
if [[ -z "$HOST_IP" ]]; then
    HOST_IP="$(ifconfig | awk '/inet / && $2 !~ /^127\./ && $2 !~ /^169\.254\./ {print $2; exit}')"
fi
[[ -n "$HOST_IP" ]] || { printf 'No private host address is available for Android Web Host testing\n' >&2; exit 2; }

HOST_COMMAND=("$BINARY" web)
if [[ "${NOLA_REAL_MOBILE_DEBUG:-0}" == "1" ]]; then
    HOST_COMMAND+=(--debug)
fi
HOST_COMMAND+=(--host 0.0.0.0 --port "$PORT" --web-root "$ROOT/apps/web" --project "$RUNTIME_PROJECT")
(
    cd "$RUNTIME_PROJECT"
    HOME="$RUNTIME_HOME" NOLA_DB_PATH="$RUNTIME_ROOT/nola.sqlite" \
        NOLA_CLI_DEBUG_LOG_PATH="$RUNTIME_ROOT/cli-debug.log" \
        exec "${HOST_COMMAND[@]}"
) >"$SERVER_LOG" 2>&1 &
SERVER_PID=$!
printf '%s\n' "$SERVER_PID" > "$RUNTIME_ROOT/host.pid"
READY_URL="http://127.0.0.1:$PORT"
BASE_URL="http://$HOST_IP:$PORT"

python3 - "$READY_URL" "$BASE_URL" "$SERVER_PID" "$DESTRUCTIVE_FILE" "$IOS_UI_SOURCE" <<'PY'
import os
import sys
import time
import urllib.request

ready_url, base_url, pid, destructive_file, ios_source = (
    sys.argv[1], sys.argv[2], int(sys.argv[3]), sys.argv[4], sys.argv[5]
)
deadline = time.monotonic() + 20
opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
while time.monotonic() < deadline:
    try:
        opener.open(ready_url + "/login", timeout=0.5).read(1)
        break
    except Exception:
        try:
            os.kill(pid, 0)
        except OSError as error:
            raise SystemExit("Nola Web Agent Host exited before readiness") from error
        time.sleep(0.1)
else:
    raise SystemExit("Nola Web Agent Host readiness timed out")

request = urllib.request.Request(
    ready_url + "/api/agent/settings",
    data=__import__("json").dumps({
        "host_tools_enabled": True,
        "enable_local_dev_tools": True,
        "network_policy": "allow",
        "allowed_dirs": [os.path.dirname(destructive_file), os.path.dirname(ios_source)],
    }).encode(),
    headers={"Content-Type": "application/json"},
    method="POST",
)
# /login only proves the HTTP listener is ready. This first product mutation
# also synchronizes the real local tool/skill catalogs in the isolated Store.
# Allow that cold transaction to finish once; never retry a timed-out mutation.
settings_started = time.monotonic()
with opener.open(request, timeout=30) as response:
    if response.status != 200:
        raise SystemExit(f"Failed to enable isolated Host tools: HTTP {response.status}")
print(f"[real-mobile-host] isolated Host settings ready in {time.monotonic() - settings_started:.2f}s")
PY

TOKEN_PATH="$(awk -F': ' '/^Nola Server credentials: / { print $2; exit }' "$SERVER_LOG")"
[[ -n "$TOKEN_PATH" ]] || {
    printf 'Nola Server credential path was not emitted by the real Host\n' >&2
    cat "$SERVER_LOG" >&2
    exit 2
}
for _ in $(seq 1 100); do
    [[ -s "$TOKEN_PATH" ]] && break
    sleep 0.05
done
[[ -s "$TOKEN_PATH" ]] || { printf 'Nola Server access token was not created\n' >&2; exit 2; }

HOST_ID="$(awk -F': ' '/^Nola Server ID: / { print $2; exit }' "$SERVER_LOG")"
[[ -n "$HOST_ID" ]] || {
    printf 'Nola Server identity was not emitted by the real Host\n' >&2
    cat "$SERVER_LOG" >&2
    exit 2
}

export NOLA_REAL_WEB_AGENT_HOST_E2E=1
export NOLA_REAL_WEB_AGENT_HOST_URL="$BASE_URL"
export NOLA_REAL_WEB_AGENT_HOST_TOKEN="$(<"$TOKEN_PATH")"
export NOLA_REAL_WEB_AGENT_HOST_ID="$HOST_ID"
export NOLA_REAL_WEB_AGENT_HOST_DESTRUCTIVE_FILE="$DESTRUCTIVE_FILE"
export NOLA_REAL_IOS_UI_SOURCE_PATH="$IOS_UI_SOURCE"
export NOLA_REAL_IOS_UI_MARKER="$IOS_UI_MARKER"
export NOLA_REAL_IOS_UI_TIMELINE_PATH="$IOS_UI_TIMELINE"
export NOLA_REAL_IOS_UI_SCREENSHOT_PATH="$IOS_UI_SCREENSHOT"
export NOLA_REAL_IOS_UI_ACCESSIBILITY_PATH="$IOS_UI_ACCESSIBILITY"
export NOLA_REAL_IOS_UI_TRANSCRIPT_PATH="$IOS_UI_TRANSCRIPT"
export IOS_TEST_STATUS_PATH
export IOS_TEST_OUTPUT

if [[ "${NOLA_SKIP_ANDROID:-0}" != "1" ]]; then
    printf '[real-mobile-host] Android production WebAgentProvider\n'
    if [[ "${NOLA_REAL_MOBILE_EXTERNAL_ONLY:-0}" == "1" ]]; then
        ANDROID_TEST_ARGS=(
            --tests com.nola.llm.provider.WebAgentProviderRealEndToEndTest.realHostRunsInstalledExternalExecutorsAndResumesTheirNativeSessions
        )
    else
        ANDROID_TEST_ARGS=(
            --tests com.nola.llm.provider.WebAgentProviderRealEndToEndTest.realHostStreamsToolLifecycleWithoutPermission
            --tests com.nola.llm.provider.WebAgentProviderRealEndToEndTest.realHostCancelsRunningToolAndRecoversConversation
            --tests com.nola.llm.provider.WebAgentProviderRealEndToEndTest.realHostPersistsAndRecallsMemoryAcrossConversations
            --tests com.nola.llm.provider.WebAgentProviderRealEndToEndTest.realHostDestructivePermissionCanBeDeniedAndResumed
            --tests com.nola.llm.provider.WebAgentProviderRealEndToEndTest.realHostRunsInstalledExternalExecutorsAndResumesTheirNativeSessions
        )
    fi
    if [[ "${NOLA_REAL_MOBILE_COMPACTION_E2E:-0}" == "1" ]]; then
        ANDROID_TEST_ARGS+=(
            --tests com.nola.llm.provider.WebAgentProviderRealEndToEndTest.realHostCompactsAndRecallsConversation
        )
    fi
    (
        cd "$ROOT/apps/android"
        JAVA_HOME="$JDK_HOME" ./gradlew :app:testDebugUnitTest --rerun-tasks \
            "${ANDROID_TEST_ARGS[@]}"
    )
    if [[ "${NOLA_REAL_MOBILE_EXTERNAL_ONLY:-0}" != "1" ]]; then
        MOBILE_RUNTIME_AUDIT="$RUNTIME_ROOT/mobile-runtime-audit.json"
        python3 "$ROOT/scripts/audit_mobile_host_runtime.py" \
            --db "$RUNTIME_ROOT/nola.sqlite" \
            --protected-file "$DESTRUCTIVE_FILE" \
            --output "$MOBILE_RUNTIME_AUDIT"
    fi
else
    printf '[real-mobile-host] Android skipped by NOLA_SKIP_ANDROID=1\n'
fi

if [[ "${NOLA_SKIP_IOS:-0}" != "1" ]]; then
    nola_xctest_device_guard_begin "$ROOT" ios_simulator
    IOS_DEVICE_ID="${NOLA_IOS_REAL_DEVICE_ID:-}"
    if [[ -z "$IOS_DEVICE_ID" ]]; then
        IOS_DEVICE_ID="$(xcrun simctl list devices available -j | python3 -c '
import json, sys
devices = [item for values in json.load(sys.stdin).get("devices", {}).values() for item in values if item.get("isAvailable") and "iPhone" in item.get("name", "")]
selected = next((item for item in devices if item.get("state") == "Booted"), None) or (devices[0] if devices else None)
if not selected: raise SystemExit("No available iPhone simulator")
print(selected["udid"])
')"
    fi

    printf '[real-mobile-host] iOS production WebAgentProvider on %s\n' "$IOS_DEVICE_ID"
    if [[ "${NOLA_SKIP_IOS_BUILD:-0}" != "1" ]]; then
        xcodebuild build-for-testing -project "$ROOT/Nola.xcodeproj" -scheme Nola \
            -destination "platform=iOS Simulator,id=$IOS_DEVICE_ID" \
            -derivedDataPath "$DERIVED_DATA" CODE_SIGNING_ALLOWED=NO -quiet
    fi
    XCTESTRUN="$(find "$DERIVED_DATA/Build/Products" -maxdepth 1 -name '*.xctestrun' -print -quit)"
    [[ -f "$XCTESTRUN" ]] || { printf 'iOS xctestrun is missing\n' >&2; exit 2; }

    python3 - "$XCTESTRUN" "$IOS_DEVICE_ID" "$READY_URL" "$NOLA_REAL_WEB_AGENT_HOST_TOKEN" "$HOST_ID" "$DESTRUCTIVE_FILE" <<'PY'
import os
import plistlib
import subprocess
import sys
import tempfile

source_path, device_id, base_url, token, host_id, destructive_file = sys.argv[1:]
fd, run_path = tempfile.mkstemp(prefix="Nola_real_host_", suffix=".xctestrun", dir=os.path.dirname(source_path))
os.close(fd)
try:
    with open(source_path, "rb") as source:
        run = plistlib.load(source)
    found = False
    ui_found = False
    for configuration in run.get("TestConfigurations", []):
        for target in configuration.get("TestTargets", []):
            blueprint = target.get("BlueprintName")
            if blueprint == "NolaUITests" and os.environ.get("NOLA_REAL_IOS_UI_E2E") == "1":
                target.setdefault("EnvironmentVariables", {}).update({
                    "NOLA_REAL_IOS_UI_E2E": "1",
                    "NOLA_REAL_WEB_AGENT_HOST_URL": base_url,
                    "NOLA_REAL_WEB_AGENT_HOST_TOKEN": token,
                    "NOLA_REAL_WEB_AGENT_HOST_ID": host_id,
                    "NOLA_REAL_LLM_MODEL": os.environ.get("NOLA_REAL_LLM_MODEL", "agent-host"),
                    "NOLA_REAL_IOS_UI_SOURCE_PATH": os.environ["NOLA_REAL_IOS_UI_SOURCE_PATH"],
                    "NOLA_REAL_IOS_UI_MARKER": os.environ["NOLA_REAL_IOS_UI_MARKER"],
                    "NOLA_REAL_IOS_UI_TIMELINE_PATH": os.environ["NOLA_REAL_IOS_UI_TIMELINE_PATH"],
                    "NOLA_REAL_IOS_UI_SCREENSHOT_PATH": os.environ["NOLA_REAL_IOS_UI_SCREENSHOT_PATH"],
                    "NOLA_REAL_IOS_UI_ACCESSIBILITY_PATH": os.environ["NOLA_REAL_IOS_UI_ACCESSIBILITY_PATH"],
                    "NOLA_REAL_IOS_UI_TRANSCRIPT_PATH": os.environ["NOLA_REAL_IOS_UI_TRANSCRIPT_PATH"],
                })
                ui_found = True
                continue
            if blueprint != "NolaTests":
                continue
            target.setdefault("EnvironmentVariables", {}).update({
                "NOLA_REAL_WEB_AGENT_HOST_E2E": "1",
                "NOLA_REAL_WEB_AGENT_HOST_URL": base_url,
                "NOLA_REAL_WEB_AGENT_HOST_TOKEN": token,
                "NOLA_REAL_WEB_AGENT_HOST_DESTRUCTIVE_FILE": destructive_file,
            })
            if os.environ.get("NOLA_REAL_MOBILE_COMPACTION_E2E") == "1":
                target["EnvironmentVariables"]["NOLA_REAL_MOBILE_COMPACTION_E2E"] = "1"
            found = True
    if not found:
        raise SystemExit("NolaTests target is missing from xctestrun")
    if os.environ.get("NOLA_REAL_IOS_UI_E2E") == "1" and not ui_found:
        raise SystemExit("NolaUITests target is missing from xctestrun")
    with open(run_path, "wb") as destination:
        plistlib.dump(run, destination)
    if os.environ.get("NOLA_REAL_IOS_UI_ONLY") == "1":
        selected_tests = []
    elif os.environ.get("NOLA_REAL_MOBILE_PROVIDER_ONLY") == "1":
        selected_tests = [
            "-only-testing:NolaTests/EndToEndFlowTests/testRealWebAgentHostStreamsToolLifecycleWithoutPermission",
        ]
    elif os.environ.get("NOLA_REAL_MOBILE_EXTERNAL_ONLY") == "1":
        selected_tests = [
            "-only-testing:NolaTests/EndToEndFlowTests/testRealWebAgentHostRunsInstalledExternalExecutorsAndResumesTheirNativeSessions",
        ]
    else:
        selected_tests = [
            "-only-testing:NolaTests/EndToEndFlowTests/testRealWebAgentHostStreamsToolLifecycleWithoutPermission",
            "-only-testing:NolaTests/EndToEndFlowTests/testRealWebAgentHostCancelsRunningToolAndRecoversConversation",
            "-only-testing:NolaTests/EndToEndFlowTests/testRealWebAgentHostPersistsAndRecallsMemoryAcrossConversations",
            "-only-testing:NolaTests/EndToEndFlowTests/testRealWebAgentHostDestructivePermissionCanBeDeniedAndResumed",
            "-only-testing:NolaTests/EndToEndFlowTests/testRealWebAgentHostRunsInstalledExternalExecutorsAndResumesTheirNativeSessions",
        ]
    if os.environ.get("NOLA_REAL_MOBILE_COMPACTION_E2E") == "1":
        selected_tests.append(
            "-only-testing:NolaTests/EndToEndFlowTests/testRealWebAgentHostCompactsAndRecallsConversation"
        )
    if os.environ.get("NOLA_REAL_IOS_UI_E2E") == "1":
        selected_tests.append(
            "-only-testing:NolaUITests/IOSRealHostChatTests/testRealHostRendersToolAndMarkdownInProductionChat"
        )
    completed = subprocess.run([
        "xcodebuild", "test-without-building",
        "-xctestrun", run_path,
        "-destination", f"platform=iOS Simulator,id={device_id}",
        "-parallel-testing-enabled", "NO",
        "-collect-test-diagnostics", "never",
        "-resultBundlePath", os.path.join(os.path.dirname(os.environ["IOS_TEST_OUTPUT"]), "ios-real.xcresult"),
        *selected_tests,
    ], check=False, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    scrubbed_output = completed.stdout.replace(token, "[REDACTED-HOST-TOKEN]")
    with open(os.environ["IOS_TEST_OUTPUT"], "w", encoding="utf-8") as destination:
        destination.write(scrubbed_output)
    print(scrubbed_output, end="")
    with open(os.environ["IOS_TEST_STATUS_PATH"], "w", encoding="utf-8") as destination:
        destination.write(str(completed.returncode))
finally:
    try: os.remove(run_path)
    except FileNotFoundError: pass
PY
else
    printf '[real-mobile-host] iOS skipped by NOLA_SKIP_IOS=1\n'
fi

if [[ "${NOLA_REAL_IOS_UI_E2E:-0}" == "1" ]]; then
    python3 - "$RUNTIME_ROOT/nola.sqlite" "$IOS_UI_MARKER" "$IOS_UI_EVENTS" "$IOS_UI_PERSISTENCE_AUDIT" "$BINARY" "$RUNTIME_HOME" "$RUNTIME_PROJECT" <<'PY'
import json
import os
import sqlite3
import subprocess
import sys
from pathlib import Path

database, marker, events_path, audit_path, binary, runtime_home, runtime_project = sys.argv[1:]
connection = sqlite3.connect(database)
connection.row_factory = sqlite3.Row
latest_run = connection.execute(
    "SELECT id AS run_id, conversation_id, assistant_message_id, status "
    "FROM runs ORDER BY updated_at_ms DESC, id DESC LIMIT 1"
).fetchone()
conversation_id = latest_run["conversation_id"] if latest_run else ""
runs = [
    dict(row) for row in connection.execute(
        "SELECT id AS run_id, conversation_id, assistant_message_id, status "
        "FROM runs WHERE conversation_id = ? ORDER BY created_at_ms, id",
        (conversation_id,),
    )
]
event_rows = list(connection.execute(
        "SELECT kind, status, event_json FROM trace_journal_events "
        "WHERE conversation_id = ? ORDER BY trace_id, sequence",
        (conversation_id,),
    ))
events = [json.loads(row["event_json"]) for row in event_rows]
tools = [
    {
        "kind": row["kind"],
        "status": row["status"],
        "payload": event.get("payload", {}),
    }
    for row, event in zip(event_rows, events)
    if row["kind"].startswith("tool.")
]
# Journal-backed Parts are hydrated by the current Core reader. Raw
# message_parts.content_text is intentionally not the content authority.
environment = dict(os.environ, HOME=runtime_home, NOLA_DB_PATH=database)
read = subprocess.run([binary, "session", "show", conversation_id, "--json"],
    cwd=runtime_project, env=environment, capture_output=True, text=True, timeout=30)
if read.returncode != 0:
    raise SystemExit("The canonical persisted session could not be read")
session = json.loads(read.stdout)
Path(audit_path).with_name("ios-canonical-session.json").write_text(read.stdout, encoding="utf-8")
projection = session["conversation_projection"]
assistant_marker_count = sum(1 for message in projection["messages"]
    if message["role"] == "assistant" and message["state"] == "completed"
    and any(part["kind"] == "text" and marker in part["markdown"] for part in message["parts"]))
canonical_tool_succeeded = any(part["kind"] == "tool" and part["capability_id"] == "read_file"
    and part["state"] == "completed" for message in projection["messages"] for part in message["parts"])
seals = {row["run_id"]: json.loads(row["seal_json"]) for row in connection.execute(
    "SELECT run_id, seal_json FROM assistant_run_terminal_seals WHERE conversation_id = ?", (conversation_id,))}
canonical_runs_completed = bool(runs) and all(row["status"] == "sealed"
    and row["run_id"] in seals
    and seals[row["run_id"]]["identity"]["conversation_id"] == conversation_id
    and seals[row["run_id"]]["identity"]["run_id"] == row["run_id"]
    and seals[row["run_id"]]["identity"]["assistant_message_id"] == row["assistant_message_id"]
    and seals[row["run_id"]]["result"]["value"]["kind"] == "answered" for row in runs)

def tool_name(event):
    payload = event.get("payload", {})
    return payload.get("tool_name")

def tool_succeeded(event):
    payload = event.get("payload", {})
    return (
        event.get("kind") == "tool.completed"
        and tool_name(event) == "read_file"
        and event.get("status") == "succeeded"
        and not payload.get("error")
    )
assertions = {
    "conversation_found": bool(conversation_id),
    "assistant_marker_persisted_once": assistant_marker_count == 1,
    "read_file_succeeded": canonical_tool_succeeded and any(tool_succeeded(event) for event in events),
    "agent_run_completed": canonical_runs_completed and projection["active_run"] is None,
    "trace_persisted": bool(events),
}
with open(events_path, "w", encoding="utf-8") as destination:
    json.dump({"conversationId": conversation_id, "events": events}, destination, ensure_ascii=False, indent=2)
with open(audit_path, "w", encoding="utf-8") as destination:
    json.dump({
        "passed": all(assertions.values()),
        "assertions": assertions,
        "runs": runs,
        "tools": tools,
    }, destination, ensure_ascii=False, indent=2)
if not all(assertions.values()):
    raise SystemExit("The canonical iOS persistence audit failed")
PY
fi

[[ "$(<"$DESTRUCTIVE_FILE")" == "KEEP_UNTIL_PERMISSION_DENIED" ]] || {
    printf 'Destructive permission denial did not preserve the protected file\n' >&2
    exit 9
}

python3 - "$ROOT" "${MOBILE_RUNTIME_AUDIT:-}" "$PROVIDER_TRACE" "$RUNTIME_ROOT/nola.sqlite" "$IOS_UI_TIMELINE" "$IOS_UI_SCREENSHOT" "$IOS_UI_ACCESSIBILITY" "$IOS_UI_TRANSCRIPT" "$IOS_UI_EVENTS" "$IOS_UI_PERSISTENCE_AUDIT" "$IOS_TEST_OUTPUT" <<'PY'
import os
from pathlib import Path
import sys

root = Path(sys.argv[1])
runtime_audit = Path(sys.argv[2]) if sys.argv[2] else None
provider_trace, runtime_db, timeline, screenshot, accessibility, transcript, ios_events, ios_audit, ios_test_output = map(Path, sys.argv[3:])
sys.path.insert(0, str(root / "scripts"))
from real_e2e_evidence import record_step

platforms = []
boundaries = ["agent-host", "provider", "persistence"]
modules = ["chat", "agent", "core", "provider", "tools", "persistence"]
assertions = {"host_chain_passed": True}
if runtime_audit is not None:
    audit = __import__("json").loads(runtime_audit.read_text(encoding="utf-8"))
    assertions["durable_runtime_audit_passed"] = audit.get("passed") is True
ui_only = os.environ.get("NOLA_REAL_IOS_UI_ONLY") == "1"
if not ui_only:
    modules.extend(["permissions", "memory", "recovery"])
    assertions["destructive_denial_preserved_file"] = True
if os.environ.get("NOLA_REAL_MOBILE_COMPACTION_E2E") == "1":
    modules.append("compaction")
    assertions["compaction_recall_passed"] = True
if os.environ.get("NOLA_SKIP_IOS") != "1":
    platforms.append("ios")
    boundaries.append("ios-ui" if os.environ.get("NOLA_REAL_IOS_UI_E2E") == "1" else "ios-provider")
    if os.environ.get("NOLA_REAL_IOS_UI_E2E") == "1":
        modules.extend(["ui", "rendering"])
        timeline_value = __import__("json").loads(timeline.read_text(encoding="utf-8")) if timeline.is_file() else []
        assertions["ios_production_chat_rendered"] = bool(timeline_value) and timeline_value[-1].get("passed") is True
        ios_audit_value = __import__("json").loads(ios_audit.read_text(encoding="utf-8")) if ios_audit.is_file() else {"passed": False}
        assertions["host_chain_passed"] = ios_audit_value.get("passed") is True
if os.environ.get("NOLA_SKIP_ANDROID") != "1":
    platforms.append("android")
    boundaries.append("android-provider")
observations = [
    {"id": "mobile-provider-traffic", "kind": "provider_trace", "path": provider_trace},
    {"id": "mobile-runtime-db", "kind": "persistence_store", "path": runtime_db},
]
assertion_evidence = {
    "host_chain_passed": ["mobile-provider-traffic", "mobile-runtime-db"],
}
if runtime_audit is not None:
    observations.append({
        "id": "mobile-runtime-audit",
        "kind": "persistence_audit",
        "path": runtime_audit,
    })
    assertion_evidence["durable_runtime_audit_passed"] = ["mobile-runtime-audit"]
if os.environ.get("NOLA_REAL_IOS_UI_E2E") == "1":
    optional_ui_observations = [
        {"id": "ios-ui-timeline", "kind": "ui_timeline", "path": timeline},
        {"id": "ios-ui-screenshot", "kind": "ui_screenshot", "path": screenshot},
        {"id": "ios-ui-accessibility", "kind": "accessibility_tree", "path": accessibility},
        {"id": "ios-ui-transcript", "kind": "transcript", "path": transcript},
        {"id": "ios-host-events", "kind": "event_stream", "path": ios_events},
        {"id": "ios-persistence-audit", "kind": "persistence_audit", "path": ios_audit},
        {"id": "ios-xctest-output", "kind": "test_report", "path": ios_test_output},
    ]
    observations.extend(item for item in optional_ui_observations if Path(item["path"]).is_file())
    assertion_evidence["ios_production_chat_rendered"] = [
        item["id"] for item in optional_ui_observations if Path(item["path"]).is_file()
    ]
if "destructive_denial_preserved_file" in assertions:
    assertion_evidence["destructive_denial_preserved_file"] = ["mobile-runtime-db"]
if "compaction_recall_passed" in assertions:
    assertion_evidence["compaction_recall_passed"] = ["mobile-runtime-db"]
for platform in platforms:
    platform_assertions = dict(assertions)
    platform_observations = list(observations)
    platform_assertion_evidence = dict(assertion_evidence)
    platform_surface_boundary = (
        "ios-ui"
        if platform == "ios" and os.environ.get("NOLA_REAL_IOS_UI_E2E") == "1"
        else f"{platform}-provider"
    )
    platform_boundaries = [
        "agent-host", "provider", "persistence", platform_surface_boundary,
    ]
    if (
        platform == "ios"
        and ios_test_output.is_file()
        and not any(item.get("id") == "ios-xctest-output" for item in platform_observations)
    ):
        platform_observations.append({
            "id": "ios-xctest-output",
            "kind": "test_report",
            "path": ios_test_output,
        })
        platform_assertion_evidence["host_chain_passed"] = [
            "mobile-provider-traffic", "mobile-runtime-db", "ios-xctest-output",
        ]
    if platform == "ios" and runtime_audit is not None:
        platform_assertions.pop("durable_runtime_audit_passed", None)
        platform_assertion_evidence.pop("durable_runtime_audit_passed", None)
        platform_observations = [
            item for item in platform_observations
            if item.get("id") != "mobile-runtime-audit"
        ]
    record_step(
        f"{platform}-agent-host-permission-memory-chain",
        platforms=[platform],
        modules=modules,
        boundaries=platform_boundaries,
        assertions=platform_assertions,
        assertion_evidence=platform_assertion_evidence,
        observations=platform_observations,
        metrics={
            "durableScenarioCount": audit.get("scenarioCount", 0)
        } if platform == "android" and runtime_audit is not None else {},
    )
PY

if [[ -s "$IOS_TEST_STATUS_PATH" ]] && [[ "$(<"$IOS_TEST_STATUS_PATH")" != "0" ]]; then
    exit "$(<"$IOS_TEST_STATUS_PATH")"
fi

printf '[real-mobile-host] passed\n'
