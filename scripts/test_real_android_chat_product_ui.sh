#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Real MainActivity + real provider Android chat acceptance.

set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
. "$ROOT/scripts/lib/real_android_device.sh"
CONFIG_PATH="${NOLA_CONFIG_PATH:-$HOME/.nola/config.json}"
ANDROID_SDK="${ANDROID_SDK_ROOT:-${ANDROID_HOME:-$HOME/Library/Android/sdk}}"
ADB="$ANDROID_SDK/platform-tools/adb"
JDK_HOME="${NOLA_ANDROID_JAVA_HOME:-/Library/Java/JavaVirtualMachines/temurin-21.jdk/Contents/Home}"
OUT="$(mktemp -d "${TMPDIR:-/tmp}/nola-android-real-ui.XXXXXX")"
READY="$OUT/provider-ready.json"; TRACE="$OUT/provider.ndjson"; CONFIG="$OUT/config.json"
TEST_USER="${NOLA_ANDROID_TEST_USER_ID:?Set a disposable secondary Android user ID}"
[[ "$TEST_USER" =~ ^[1-9][0-9]*$ ]] || { echo "Refusing primary or invalid Android test user" >&2; exit 2; }
python3 - "$CONFIG_PATH" "$CONFIG" <<'CONFIG_PY'
import json, os, sys
from pathlib import Path
llm = json.loads(Path(sys.argv[1]).read_text())["llm"]
selection = llm["default_model"]
connections = [item for item in llm["connections"] if item["id"] == selection["connection"]]
if len(connections) != 1:
    raise SystemExit("Default model must reference exactly one configured connection")
connection = connections[0]
model = selection["model"]
if not any(item.get("id") == model for item in connection["models"]):
    raise SystemExit("Default model is not in the selected connection")
credential = os.environ.get(connection["api_key_env"])
if not credential:
    raise SystemExit("Configured credential environment is unavailable")
Path(sys.argv[2]).write_text(json.dumps({"endpoint": connection["endpoint"], "apiKey": credential, "model": model, "protocol": connection["protocol"]}))
os.chmod(sys.argv[2], 0o600)
CONFIG_PY
python3 "$ROOT/scripts/e2e/provider_probe.py" --capture-last-user-message --upstream "$(jq -er '.endpoint' "$CONFIG")" --log "$TRACE" --ready "$READY" &
PROBE_PID=$!
ORIGINAL_USER=""
ORIGINAL_SCREEN_TIMEOUT=""
cleanup() {
    kill -TERM "$PROBE_PID" 2>/dev/null || true
    wait "$PROBE_PID" 2>/dev/null || true
    if [[ -n "${SERIAL:-}" ]]; then
        [[ -z "$ORIGINAL_SCREEN_TIMEOUT" ]] || "$ADB" -s "$SERIAL" shell settings --user "$TEST_USER" put system screen_off_timeout "$ORIGINAL_SCREEN_TIMEOUT" >/dev/null 2>&1 || true
        "$ADB" -s "$SERIAL" shell run-as com.nola --user "$TEST_USER" rm -f files/nola-real-llm-e2e.json >/dev/null 2>&1 || true
        [[ -z "$ORIGINAL_USER" ]] || "$ADB" -s "$SERIAL" shell am switch-user "$ORIGINAL_USER" >/dev/null 2>&1 || true
    fi
    rm -f "$CONFIG"
    real_android_release_device
    printf 'Android real UI evidence: %s\n' "$OUT"
}
trap cleanup EXIT INT TERM
for _ in {1..100}; do [[ -s "$READY" ]] && break; sleep 0.05; done
[[ -s "$READY" ]] || exit 2
REAL_ANDROID_ADB="$ADB"; REAL_ANDROID_EMULATOR="$ANDROID_SDK/emulator/emulator"; real_android_require_device
SERIAL="$REAL_ANDROID_DEVICE_ID"; export NOLA_ANDROID_ABIS="$REAL_ANDROID_ABI"
PROXY_ENDPOINT="$(jq -er '.endpoint' "$READY")"
PORT="$(python3 - "$PROXY_ENDPOINT" <<'PY'
from urllib.parse import urlsplit
import sys
port = urlsplit(sys.argv[1]).port
if port is None:
    raise SystemExit('provider probe endpoint has no explicit port')
print(port)
PY
)"
"$ADB" -s "$SERIAL" reverse "tcp:$PORT" "tcp:$PORT"
python3 - "$CONFIG" "$PROXY_ENDPOINT" <<'CONFIG_PY'
import json, sys
from pathlib import Path
path = Path(sys.argv[1]); config = json.loads(path.read_text())
config["endpoint"] = sys.argv[2]; path.write_text(json.dumps(config))
CONFIG_PY
ORIGINAL_USER="$("$ADB" -s "$SERIAL" shell am get-current-user | tr -d '\r')"
"$ADB" -s "$SERIAL" shell am start-user "$TEST_USER" >/dev/null
"$ADB" -s "$SERIAL" shell am switch-user "$TEST_USER"
[[ "$("$ADB" -s "$SERIAL" shell am get-current-user | tr -d '\r')" == "$TEST_USER" ]] || exit 2

(cd "$ROOT/android" && JAVA_HOME="$JDK_HOME" ./gradlew :app:assembleDebug :app:assembleDebugAndroidTest --no-daemon --quiet)
"$ADB" -s "$SERIAL" install --user "$TEST_USER" -r "$ROOT/android/app/build/outputs/apk/debug/app-debug.apk" >/dev/null
"$ADB" -s "$SERIAL" install --user "$TEST_USER" -r "$ROOT/android/app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk" >/dev/null
ORIGINAL_SCREEN_TIMEOUT="$("$ADB" -s "$SERIAL" shell settings --user "$TEST_USER" get system screen_off_timeout | tr -d '\r')"
"$ADB" -s "$SERIAL" shell settings --user "$TEST_USER" put system screen_off_timeout 1800000
"$ADB" -s "$SERIAL" shell input keyevent 224
"$ADB" -s "$SERIAL" shell wm dismiss-keyguard
"$ADB" -s "$SERIAL" shell run-as com.nola --user "$TEST_USER" mkdir -p files
"$ADB" -s "$SERIAL" shell "run-as com.nola --user $TEST_USER sh -c 'cat > files/nola-real-llm-e2e.json'" < "$CONFIG"
"$ADB" -s "$SERIAL" logcat -c
set +e
"$ADB" -s "$SERIAL" shell am instrument --user "$TEST_USER" -w -r -e isolatedUserId "$TEST_USER" -e class 'com.nola.ui.screen.chat.RealAndroidChatProductUiEndToEndTest#realMainActivityStreamsPersistsAndRendersRealProviderAnswer' com.nola.test/androidx.test.runner.AndroidJUnitRunner | tee "$OUT/instrumentation.log"
STATUS=${PIPESTATUS[0]}
set -e
"$ADB" -s "$SERIAL" logcat -d -v threadtime > "$OUT/logcat.txt" || true
"$ADB" -s "$SERIAL" shell am force-stop --user "$TEST_USER" com.nola
for name in real-model-selection-failure.png real-model-selection-failure.txt real-android-ui.png real-android-ui-accessibility.txt real-android-ui-timeline.json real-android-ui-transcript.json; do
    remote="/data/user/$TEST_USER/com.nola/files/$name"
    if "$ADB" -s "$SERIAL" shell run-as com.nola --user "$TEST_USER" test -f "$remote"; then
        "$ADB" -s "$SERIAL" exec-out run-as com.nola --user "$TEST_USER" cat "$remote" > "$OUT/$name"
    fi
done
CANONICAL_DB=/data/user/$TEST_USER/com.nola/no_backup/nola-product.db
for suffix in '' '-wal' '-shm'; do
    remote="$CANONICAL_DB$suffix"
    if "$ADB" -s "$SERIAL" shell run-as com.nola --user "$TEST_USER" test -f "$remote"; then
        "$ADB" -s "$SERIAL" exec-out run-as com.nola --user "$TEST_USER" cat "$remote" > "$OUT/nola-product.db$suffix"
    fi
done
python3 - "$ROOT" "$OUT" "$STATUS" <<'PY'
import json, sqlite3, sys
from pathlib import Path
root, out, status = Path(sys.argv[1]), Path(sys.argv[2]), int(sys.argv[3])
sys.path.insert(0, str(root/'scripts'))
from real_e2e_evidence import record_step
marker='ANDROID-UI-REAL-741963'
required=[
 out/'real-android-ui.png', out/'real-android-ui-accessibility.txt',
 out/'real-android-ui-timeline.json', out/'real-android-ui-transcript.json',
 out/'nola-product.db',
]
missing=[str(path) for path in required if not path.is_file() or path.stat().st_size == 0]
if missing:
 raise SystemExit('missing real Android evidence: '+', '.join(missing))
if (out/'nola-product.db').read_bytes()[:16] != b'SQLite format 3\x00':
 raise SystemExit('canonical Android Store is not SQLite evidence')
db=sqlite3.connect(out/'nola-product.db')
found=False
for (table,) in db.execute("select name from sqlite_master where type='table' and name not like 'sqlite_%'"):
 for row in db.execute(f'SELECT * FROM "{table}"'):
  if marker in ' '.join('' if v is None else str(v) for v in row): found=True
db.close()
provider_events=[json.loads(line) for line in (out/'provider.ndjson').read_text().splitlines() if line.strip()]
provider_response=next((event for event in reversed(provider_events) if event.get('event') == 'provider.response'), {})
provider_streamed=(
 provider_response.get('status') == 200 and
 provider_response.get('stream') is True and
 int(provider_response.get('responseTextDeltaCount') or 0) >= 2 and
 'QU5EUk9JRC1VSS1SRUFMLTc0MTk2Mw==' in str(provider_response.get('lastUserMessage') or '') and
 'FINISH-369147' in str(provider_response.get('lastUserMessage') or '')
)
audit={'passed':found and provider_streamed,'assertions':{
 'assistant_marker_persisted':found,
 'provider_returned_multiple_text_deltas':provider_streamed,
}}
(out/'persistence-audit.json').write_text(json.dumps(audit,indent=2))
instrumented=status==0 and 'OK (1 test)' in (out/'instrumentation.log').read_text(errors='replace')
record_step('android-main-activity-real-chat',platforms=['android'],modules=['chat','agent','provider','ui','persistence'],boundaries=['android-ui','android-client-runtime','provider','persistence'],assertions={'main_activity_rendered_real_answer':instrumented,'assistant_marker_persisted':found,'provider_returned_multiple_text_deltas':provider_streamed},assertion_evidence={'main_activity_rendered_real_answer':['instrumentation','screenshot','accessibility','timeline'],'assistant_marker_persisted':['persistence-audit','database'],'provider_returned_multiple_text_deltas':['provider']},observations=[
 {'id':'provider','kind':'provider_trace','path':out/'provider.ndjson'},
 {'id':'instrumentation','kind':'instrumentation_output','path':out/'instrumentation.log'},
 {'id':'screenshot','kind':'ui_screenshot','path':out/'real-android-ui.png'},
 {'id':'accessibility','kind':'accessibility_tree','path':out/'real-android-ui-accessibility.txt'},
 {'id':'timeline','kind':'ui_timeline','path':out/'real-android-ui-timeline.json'},
 {'id':'transcript','kind':'transcript','path':out/'real-android-ui-transcript.json'},
 {'id':'database','kind':'persistence_store','path':out/'nola-product.db'},
 {'id':'persistence-audit','kind':'persistence_audit','path':out/'persistence-audit.json'},
],metrics={'instrumentExitCode':status})
if not instrumented or not found or not provider_streamed: raise SystemExit(9)
PY
