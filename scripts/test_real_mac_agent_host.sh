#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Real-provider Mac Agent Host acceptance test. This is intentionally opt-in
# and reads the user's existing ~/.nola/config.json without printing secrets.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
source "$ROOT/scripts/e2e/cargo_paths.sh"
REAL_USER_HOME="$HOME"
PRODUCTION_DB="$REAL_USER_HOME/Library/Application Support/Nola/State/nola-v4.db"
if [[ -n "${NOLA_MAC_REAL_DERIVED_DATA:-}" ]]; then
    DERIVED_DATA="$NOLA_MAC_REAL_DERIVED_DATA"
    mkdir -p "$DERIVED_DATA"
else
    # Product builds are expensive (MLX Swift includes large git submodules).
    # Reuse compiler/package artifacts across scenarios while keeping every
    # runtime database and workspace isolated below.
    DERIVED_DATA="$ROOT/.build/real-mac-derived-data"
    mkdir -p "$DERIVED_DATA"
fi
CONFIG_PATH="${NOLA_CONFIG_PATH:-$HOME/.nola/config.json}"
MAC_ARCH="$(uname -m)"
APP="$DERIVED_DATA/Build/Products/Test/Nola.app/Contents/MacOS/Nola"
APP_BUNDLE="$DERIVED_DATA/Build/Products/Test/Nola.app"
APP_DEBUG_CODE="$APP_BUNDLE/Contents/MacOS/Nola.debug.dylib"
EMBEDDED_HOST="$APP_BUNDLE/Contents/MacOS/nola-agent-host"
if [[ -n "${NOLA_MAC_REAL_WORKSPACE:-}" ]]; then
    WORKSPACE="$NOLA_MAC_REAL_WORKSPACE"
    mkdir -p "$WORKSPACE"
else
    WORKSPACE="$(mktemp -d "${TMPDIR:-/tmp}/nola-mac-real-e2e.XXXXXX")"
fi
ARTIFACT_ROOT="${NOLA_MAC_REAL_ARTIFACT_ROOT:-$WORKSPACE}"
mkdir -p "$ARTIFACT_ROOT"
RESULT="$ARTIFACT_ROOT/result.json"
RUNTIME_LOG="$ARTIFACT_ROOT/runtime.log"
SESSION_ID="mac-runtime-acceptance-$(uuidgen | tr '[:upper:]' '[:lower:]')"
RUNTIME_SUPPORT="$ARTIFACT_ROOT/ApplicationSupport/Nola"
RUNTIME_CACHE="$ARTIFACT_ROOT/Caches/Nola"
RUNTIME_DB="$RUNTIME_SUPPORT/State/nola-v4.db"
RUNTIME_HOME="$RUNTIME_SUPPORT/Runtime/Home"
mkdir -p "$RUNTIME_SUPPORT/State" "$RUNTIME_CACHE" "$RUNTIME_HOME"
PROMPT="${NOLA_MAC_REAL_PROMPT-Read note.txt using the available file tools. Then answer with exactly NOLA_MAC_TOOL_REAL_OK and nothing else.}"
EXPECTED_EXACT="${NOLA_MAC_REAL_EXPECTED_EXACT-NOLA_MAC_TOOL_REAL_OK}"
EXPECTED_CONTAINS="${NOLA_MAC_REAL_EXPECTED_CONTAINS-}"
MIN_ASSISTANT_DELTAS="${NOLA_MAC_REAL_MIN_ASSISTANT_DELTAS:-1}"
REQUIRE_TOOL="${NOLA_MAC_REAL_REQUIRE_TOOL:-1}"
SCREENSHOT="${NOLA_MAC_REAL_SCREENSHOT:-}"
CONTEXT="${NOLA_MAC_REAL_CONTEXT:-}"
APP_PID=""
HOST_PID=""
PROVIDER_PROBE_PID=""
PROVIDER_TRACE="$ARTIFACT_ROOT/provider.ndjson"
PROVIDER_READY="$ARTIFACT_ROOT/provider.endpoint"

cleanup() {
    if [[ -n "$PROVIDER_PROBE_PID" ]]; then
        kill -TERM "$PROVIDER_PROBE_PID" 2>/dev/null || true
        wait "$PROVIDER_PROBE_PID" 2>/dev/null || true
    fi
    if [[ -n "$HOST_PID" ]]; then
        kill -TERM "$HOST_PID" 2>/dev/null || true
        wait "$HOST_PID" 2>/dev/null || true
    fi
    if [[ -n "$APP_PID" ]]; then
        kill -TERM "$APP_PID" 2>/dev/null || true
        wait "$APP_PID" 2>/dev/null || true
    fi
    # Keep isolated reports, databases and provider evidence after failure too.
}
trap cleanup EXIT INT TERM

[[ -f "$CONFIG_PATH" ]] || {
    printf 'Missing Nola config: %s\n' "$CONFIG_PATH" >&2
    exit 2
}
command -v node >/dev/null 2>&1 || {
    printf 'Node.js is required to read and validate JSON without exposing credentials.\n' >&2
    exit 2
}

read_llm_setting() {
    node - "$CONFIG_PATH" "$1" <<'NODE'
const fs = require("fs");
const value = JSON.parse(fs.readFileSync(process.argv[2], "utf8"));
const llm = value.llm || {};
const key = process.argv[3];
const item = typeof llm[key] === "string" ? llm[key].trim() : "";
if (!item) process.exit(3);
process.stdout.write(item);
NODE
}
LLM_ENDPOINT="${NOLA_REAL_E2E_ENDPOINT:-${MODEL_API_URL:-$(read_llm_setting endpoint)}}" || {
    printf 'Nola config must define llm.endpoint, llm.api_key, and llm.model.\n' >&2
    exit 2
}
LLM_API_KEY="${NOLA_REAL_E2E_API_KEY:-${MODEL_API_KEY:-$(read_llm_setting api_key)}}" || exit 2
LLM_MODEL="${NOLA_REAL_E2E_MODEL:-${MODEL_NAME:-$(read_llm_setting model)}}" || exit 2
MODEL_FIXTURE="$(python3 "$ROOT/scripts/e2e/model_execution_fixture.py" \
    --config "$CONFIG_PATH" --model "$LLM_MODEL" --endpoint "$LLM_ENDPOINT" \
    --require-tool "$REQUIRE_TOOL")" || exit 2
CANONICAL_READER="${NOLA_AUDIT_CANONICAL_READER:-$(nola_cargo_example "$ROOT" debug \
    audit_canonical_text_reader "$ROOT/core/nola-application-bootstrap/Cargo.toml")}"
[[ -x "$CANONICAL_READER" ]] || {
    printf 'Build the canonical audit reader before real acceptance: cargo build --manifest-path %s -p nola_application_bootstrap --example audit_canonical_text_reader\n' \
        "$ROOT/Cargo.toml" >&2
    printf 'Or set NOLA_AUDIT_CANONICAL_READER to that built example executable.\n' >&2
    exit 2
}

python3 "$ROOT/scripts/e2e/provider_probe.py" \
    --upstream "$LLM_ENDPOINT" \
    --log "$PROVIDER_TRACE" \
    --ready "$PROVIDER_READY" \
    >"$ARTIFACT_ROOT/provider-probe.log" 2>&1 &
PROVIDER_PROBE_PID=$!
for _ in {1..100}; do
    [[ -s "$PROVIDER_READY" ]] && break
    kill -0 "$PROVIDER_PROBE_PID" 2>/dev/null || break
    sleep 0.05
done
[[ -s "$PROVIDER_READY" ]] || {
    printf 'Mac provider evidence probe failed to start.\n' >&2
    cat "$ARTIFACT_ROOT/provider-probe.log" >&2 || true
    exit 2
}
LLM_ACCEPTANCE_ENDPOINT="$(node - "$PROVIDER_READY" <<'NODE'
const fs = require("fs");
const value = JSON.parse(fs.readFileSync(process.argv[2], "utf8"));
if (typeof value.endpoint !== "string" || !value.endpoint.trim()) process.exit(3);
process.stdout.write(value.endpoint.trim());
NODE
)" || {
    printf 'Mac provider evidence probe returned an invalid endpoint.\n' >&2
    exit 2
}

if [[ "${NOLA_SKIP_MAC_BUILD:-0}" != "1" ]]; then
    printf '[real-mac-agent] building NolaMac\n'
    if ! xcodebuild build \
        -project "$ROOT/Nola.xcodeproj" \
        -scheme NolaMac \
        -configuration Test \
        -destination "platform=macOS,arch=$MAC_ARCH" \
        -derivedDataPath "$DERIVED_DATA" \
        ARCHS="$MAC_ARCH" \
        ONLY_ACTIVE_ARCH=YES \
        CODE_SIGNING_ALLOWED=NO \
        >"$ARTIFACT_ROOT/build.log" 2>&1; then
        printf '[real-mac-agent] build failed; diagnostics: %s\n' "$ARTIFACT_ROOT/build.log" >&2
        rg -n -C 4 '(^|[^A-Za-z])error:|BUILD FAILED|Command .* failed|PhaseScriptExecution' \
            "$ARTIFACT_ROOT/build.log" 2>/dev/null | tail -n 120 >&2 || true
        exit 65
    fi
fi
codesign --force --deep --sign - "$APP_BUNDLE" >/dev/null
[[ -x "$APP" ]] || {
    printf 'NolaMac executable is missing: %s\n' "$APP" >&2
    exit 2
}
[[ -x "$EMBEDDED_HOST" ]] || {
    printf 'Embedded Agent Host is missing: %s\n' "$EMBEDDED_HOST" >&2
    exit 2
}
APP_CODE="$APP"
if [[ -f "$APP_DEBUG_CODE" ]]; then
    APP_CODE="$APP_DEBUG_CODE"
fi
if [[ "${NOLA_SKIP_MAC_BUILD:-0}" == "1" ]]; then
    STALE_SWIFT_SOURCE="$(find "$ROOT/apps/macos" "$ROOT/apps/ios/Services" "$ROOT/apps/ios/LLM" \
        -type f -name '*.swift' -newer "$APP_CODE" -print -quit 2>/dev/null || true)"
    STALE_HOST_SOURCE="$(find "$ROOT/core/src" "$ROOT/apps/cli/src" \
        -type f \( -name '*.rs' -o -name '*.toml' \) -newer "$EMBEDDED_HOST" \
        -print -quit 2>/dev/null || true)"
    if [[ -n "$STALE_SWIFT_SOURCE" || -n "$STALE_HOST_SOURCE" ]]; then
        printf 'Refusing stale Mac acceptance artifacts. Rebuild before testing.\n' >&2
        [[ -z "$STALE_SWIFT_SOURCE" ]] || printf 'Newer Swift source: %s\n' "$STALE_SWIFT_SOURCE" >&2
        [[ -z "$STALE_HOST_SOURCE" ]] || printf 'Newer Host source: %s\n' "$STALE_HOST_SOURCE" >&2
        exit 2
    fi
fi
APP_SHA256="$(shasum -a 256 "$APP" | awk '{print $1}')"
APP_CODE_SHA256="$(shasum -a 256 "$APP_CODE" | awk '{print $1}')"
HOST_SHA256="$(shasum -a 256 "$EMBEDDED_HOST" | awk '{print $1}')"

if [[ "${NOLA_MAC_REAL_SEED_FIXTURE:-1}" == "1" ]]; then
printf 'NOLA_MAC_TOOL_REAL_OK\n' >"$WORKSPACE/note.txt"
cat >"$WORKSPACE/architecture.md" <<'EOF'
# Runtime presentation

The product must preserve a stable committed transcript while the mutable
stream tail is updated. Tool calls and thinking are visible process records,
not assistant prose, and internal control tools must remain hidden.
EOF
cat >"$WORKSPACE/sample.swift" <<'EOF'
struct StableStream {
    let committedPrefix: String
    var mutableTail: String
}
EOF
fi
printf '[real-mac-agent] running isolated real-provider tool turn\n'
NOLA_MAC_RUNTIME_ACCEPTANCE="$RESULT" \
NOLA_MAC_RUNTIME_ACCEPTANCE_PROJECT="$WORKSPACE" \
NOLA_MAC_RUNTIME_ACCEPTANCE_PROMPT="$PROMPT" \
NOLA_MAC_RUNTIME_ACCEPTANCE_CONTEXT="$CONTEXT" \
NOLA_MAC_RUNTIME_ACCEPTANCE_REQUIRE_TOOL="$REQUIRE_TOOL" \
NOLA_MAC_RUNTIME_ACCEPTANCE_REQUIRE_USAGE="${NOLA_MAC_REAL_REQUIRE_USAGE:-0}" \
NOLA_MAC_RUNTIME_ACCEPTANCE_REQUIRE_THINKING="${NOLA_MAC_REAL_REQUIRE_THINKING:-0}" \
NOLA_MAC_RUNTIME_ACCEPTANCE_CANCELLATION_PROMPT="${NOLA_MAC_REAL_CANCELLATION_PROMPT:-}" \
NOLA_MAC_RUNTIME_ACCEPTANCE_PERMISSION_DECISION="${NOLA_MAC_REAL_PERMISSION_DECISION:-allow}" \
NOLA_MAC_RUNTIME_ACCEPTANCE_PERMISSION_MODE="${NOLA_MAC_REAL_PERMISSION_MODE:-cautious}" \
NOLA_MAC_RUNTIME_ACCEPTANCE_PERMISSION_FIXTURE="${NOLA_MAC_REAL_PERMISSION_FIXTURE:-}" \
NOLA_MAC_RUNTIME_ACCEPTANCE_PERMISSION_SCOPE="${NOLA_MAC_REAL_PERMISSION_SCOPE:-once}" \
NOLA_MAC_RUNTIME_ACCEPTANCE_ENDPOINT="$LLM_ACCEPTANCE_ENDPOINT" \
NOLA_MAC_RUNTIME_ACCEPTANCE_API_KEY="$LLM_API_KEY" \
NOLA_MAC_RUNTIME_ACCEPTANCE_MODEL="$LLM_MODEL" \
NOLA_MAC_RUNTIME_ACCEPTANCE_MODEL_FIXTURE_JSON="$MODEL_FIXTURE" \
NOLA_MAC_RUNTIME_ACCEPTANCE_REASONING_JSON="${NOLA_MAC_REAL_REASONING_JSON:-}" \
NOLA_MAC_RUNTIME_ACCEPTANCE_VISION_INPUT="${NOLA_MAC_REAL_VISION_INPUT:-0}" \
NOLA_MAC_RUNTIME_ACCEPTANCE_SCREENSHOT="$SCREENSHOT" \
NOLA_MAC_RUNTIME_ACCEPTANCE_WEB_SOURCE_URL="${NOLA_MAC_RUNTIME_ACCEPTANCE_WEB_SOURCE_URL:-}" \
NOLA_MAC_RUNTIME_ACCEPTANCE_AUTOMATION_CHAIN="${NOLA_MAC_RUNTIME_ACCEPTANCE_AUTOMATION_CHAIN:-0}" \
NOLA_MAC_RUNTIME_ACCEPTANCE_AUTOMATION_OUTPUT="${NOLA_MAC_RUNTIME_ACCEPTANCE_AUTOMATION_OUTPUT:-}" \
NOLA_MAC_RUNTIME_ACCEPTANCE_AUTOMATION_SOURCE_TOKEN="${NOLA_MAC_RUNTIME_ACCEPTANCE_AUTOMATION_SOURCE_TOKEN:-}" \
NOLA_MAC_RUNTIME_ACCEPTANCE_FOLLOW_UP_PROMPT="${NOLA_MAC_REAL_FOLLOW_UP_PROMPT:-}" \
NOLA_MAC_RUNTIME_ACCEPTANCE_FOLLOW_UP_EXPECTED="${NOLA_MAC_REAL_FOLLOW_UP_EXPECTED:-}" \
NOLA_MAC_RUNTIME_ACCEPTANCE_SESSION_ID="$SESSION_ID" \
NOLA_MAC_RUNTIME_ACCEPTANCE_EXECUTION_BACKEND="${NOLA_MAC_REAL_EXECUTION_BACKEND:-nola}" \
NOLA_HOME="$RUNTIME_HOME" \
XDG_CONFIG_HOME="$RUNTIME_SUPPORT/Configuration" \
XDG_DATA_HOME="$RUNTIME_SUPPORT/Data" \
XDG_CACHE_HOME="$RUNTIME_CACHE" \
NOLA_DB_PATH="$RUNTIME_DB" \
NOLA_MAC_APP_SUPPORT="$RUNTIME_SUPPORT" \
NOLA_MAC_CACHE_ROOT="$RUNTIME_CACHE" \
NOLA_TEST_MODE=1 \
NOLA_REQUIRE_MAC_SYSTEM_PERMISSIONS="$([[ -n "$SCREENSHOT" ]] && printf 1 || printf 0)" \
"$APP" >"$RUNTIME_LOG" 2>&1 &
APP_PID=$!

WAIT_TICKS="${NOLA_MAC_REAL_WAIT_TICKS:-}"
if [[ -z "$WAIT_TICKS" ]]; then
    if [[ "${NOLA_MAC_RUNTIME_ACCEPTANCE_AUTOMATION_CHAIN:-0}" == "1" ]]; then
        WAIT_TICKS=3000
    else
        WAIT_TICKS=1200
    fi
fi
for (( tick = 0; tick < WAIT_TICKS; tick++ )); do
    [[ -s "$RESULT" ]] && break
    kill -0 "$APP_PID" 2>/dev/null || break
    HOST_PID="$(pgrep -P "$APP_PID" -f 'nola-agent-host run' | head -n 1 || true)"
    sleep 0.1
done

if [[ ! -s "$RESULT" ]]; then
    node - "$RUNTIME_LOG" "$RESULT.events.jsonl" "$APP_PID" "$HOST_PID" <<'NODE'
const fs = require("fs");
const tail = fs.readFileSync(process.argv[2], "utf8").split(/\n/).slice(-12);
const eventTail = fs.existsSync(process.argv[3])
  ? fs.readFileSync(process.argv[3], "utf8").split(/\n/).filter(Boolean).slice(-12).map(line => JSON.parse(line))
  : [];
console.log(JSON.stringify({
  producedResult: false,
  appPid: process.argv[4],
  hostPid: process.argv[5] || null,
  logTail: tail,
  eventTail
}));
NODE
    exit 8
fi

# A failure receipt is terminal for acceptance, even if AppKit shutdown is
# waiting for a stalled Host. Never let an unbounded wait hide that failure.
for (( shutdown_tick = 0; shutdown_tick < 200; shutdown_tick++ )); do
    kill -0 "$APP_PID" 2>/dev/null || break
    sleep 0.1
done
if kill -0 "$APP_PID" 2>/dev/null; then
    kill -TERM "$APP_PID" 2>/dev/null || true
    for (( shutdown_tick = 0; shutdown_tick < 50; shutdown_tick++ )); do
        kill -0 "$APP_PID" 2>/dev/null || break
        sleep 0.1
    done
    kill -0 "$APP_PID" 2>/dev/null && kill -KILL "$APP_PID" 2>/dev/null || true
fi
wait "$APP_PID" 2>/dev/null || true
APP_PID=""
sleep 0.2
[[ -s "$RUNTIME_DB" ]] || {
    printf 'Isolated Agent Host database was not created: %s\n' "$RUNTIME_DB" >&2
    exit 9
}
if [[ -f "$PRODUCTION_DB" ]] \
    && [[ "$(sqlite3 -readonly "$PRODUCTION_DB" \
        "SELECT count(*) FROM conversations WHERE id = replace('$SESSION_ID', '''', '''''');" \
        2>/dev/null || printf 0)" != "0" ]]; then
    printf 'Real Mac acceptance leaked into the production database: %s\n' "$SESSION_ID" >&2
    exit 9
fi
SESSION_ROW_COUNT=""
for _ in {1..40}; do
    SESSION_ROW_COUNT="$(sqlite3 -readonly -cmd '.timeout 1000' "$RUNTIME_DB" \
        "SELECT count(*) FROM conversations WHERE id = replace('$SESSION_ID', '''', '''''');" \
        2>/dev/null || true)"
    [[ "$SESSION_ROW_COUNT" == "1" ]] && break
    sleep 0.05
done
if [[ "$SESSION_ROW_COUNT" != "1" ]]; then
    printf 'Isolated Agent Host database does not own the acceptance session: %s\n' \
        "$SESSION_ID" >&2
    exit 9
fi
HOST_ALIVE=false
if [[ -n "$HOST_PID" ]] && kill -0 "$HOST_PID" 2>/dev/null; then
    HOST_ALIVE=true
fi

RUNTIME_ACCEPTANCE_FAILED=0
if ! node - "$RESULT" "$HOST_ALIVE" "$EXPECTED_EXACT" "$EXPECTED_CONTAINS" "$MIN_ASSISTANT_DELTAS" "$REQUIRE_TOOL" "$SCREENSHOT" <<'NODE'; then
const fs = require("fs");
const value = JSON.parse(fs.readFileSync(process.argv[2], "utf8"));
const events = value.events || [];
// Trace is the audit/progress log. The harness independently reconstructs live
// Core Parts and reloads the persisted final body; compare those two bodies.
const assistantText = String(value.render_probe?.raw_text || "");
const count = kind => events.filter(event => event.kind === kind).length;
const expectedExact = process.argv[4];
const expectedParts = process.argv[5]
  ? process.argv[5].split("|||").map(value => value.trim()).filter(Boolean)
  : [];
const minimumAssistantDeltas = Number(process.argv[6] || 1);
const requireTool = process.argv[7] !== "0";
const screenshotRequested = Boolean(process.argv[8]);
const maximumToolErrors = Number(process.env.NOLA_MAC_REAL_MAX_TOOL_ERRORS || 0);
const maximumCompletedTools = Number(process.env.NOLA_MAC_REAL_MAX_COMPLETED_TOOLS || -1);
const maximumThinkingDeltas = Number(process.env.NOLA_MAC_REAL_MAX_THINKING_DELTAS || 0);
const maximumAssistantResets = Number(process.env.NOLA_MAC_REAL_MAX_ASSISTANT_RESETS || 1);
const minimumMarkdownNodes = Number(process.env.NOLA_MAC_REAL_MIN_MARKDOWN_NODES || 0);
const minimumRenderedTables = Number(process.env.NOLA_MAC_REAL_MIN_RENDERED_TABLES || 0);
const minimumCodeNodes = Number(process.env.NOLA_MAC_REAL_MIN_CODE_NODES || 0);
const minimumMutableTailUpdates = Number(process.env.NOLA_MAC_REAL_MIN_MUTABLE_TAIL_UPDATES || 0);
const minimumVisibleTools = Number(process.env.NOLA_MAC_REAL_MIN_VISIBLE_TOOLS || (requireTool ? 1 : 0));
const expectedPermissionRequests = Number(process.env.NOLA_MAC_REAL_EXPECTED_PERMISSION_REQUESTS || -1);
const minimumPermissionRequests = Number(process.env.NOLA_MAC_REAL_MIN_PERMISSION_REQUESTS || 0);
const forbiddenParts = String(process.env.NOLA_MAC_REAL_FORBIDDEN_CONTAINS || "")
  .split("|||").map(value => value.trim()).filter(Boolean);
const expectedTools = String(process.env.NOLA_MAC_REAL_EXPECTED_TOOLS || "")
  .split(",").map(value => value.trim()).filter(Boolean);
const followUpExpectedPattern = String(process.env.NOLA_MAC_REAL_FOLLOW_UP_EXPECTED_REGEX || "").trim();
const followUpForbiddenParts = String(process.env.NOLA_MAC_REAL_FOLLOW_UP_FORBIDDEN_CONTAINS || "")
  .split("|||").map(value => value.trim()).filter(Boolean);
const followUpMaximumTools = Number(process.env.NOLA_MAC_REAL_FOLLOW_UP_MAX_TOOLS || -1);
let followUpPattern = null;
try {
  followUpPattern = followUpExpectedPattern ? new RegExp(followUpExpectedPattern, "u") : null;
} catch (error) {
  console.error(`Invalid follow-up regex: ${error}`);
  process.exit(9);
}
const renderProbe = value.render_probe || {};
const screenshot = renderProbe.screenshot || {};
const toolEvents = events
  .filter(event => event.kind === "toolStarted" || event.kind === "toolFinished")
  .map(event => ({
    kind: event.kind,
    tool: event.tool_name || event.tool || event.name || event.title || null,
    callId: event.tool_call_id || event.call_id || event.callId || null,
    isError: event.is_error === true,
    detail: String(event.detail || event.error || event.message || "").slice(0, 500),
  }));
const report = {
  success: value.success === true,
  eventCount: events.length,
  toolStarted: count("toolStarted"),
  toolFinished: count("toolFinished"),
  toolErrors: events.filter(event => event.kind === "toolFinished" && event.is_error === true).length,
  assistantResets: count("assistantReset"),
  assistantDeltas: count("assistantDelta"),
  coreTextProjections: Number(renderProbe.core_text_projection_count || 0),
  bodyMutations: Number(renderProbe.body_mutation_count || 0),
  thinkingDeltas: count("thinkingDelta"),
  permissionRequests: count("permissionRequested"),
  permissionCardsPresented: Number(value.permission_cards_presented || 0),
  assistantText,
  toolEvents,
  renderProbe,
  mutableTailUpdates: Number(renderProbe.mutable_tail_update_count || 0),
  followUp: value.follow_up || {},
  screenshot,
  finished: count("finished"),
  hostAlive: process.argv[3] === "true"
};
const completedTools = new Set(toolEvents
  .filter(event => event.kind === "toolFinished" && !event.isError)
  .map(event => event.tool));
const startedCallIDs = new Set(toolEvents
  .filter(event => event.kind === "toolStarted")
  .map(event => event.callId).filter(Boolean));
const finishedCallIDs = new Set(toolEvents
  .filter(event => event.kind === "toolFinished")
  .map(event => event.callId).filter(Boolean));
const failedAssertions = [];
const requireAssertion = (name, condition) => {
  if (!condition) failedAssertions.push(name);
};
requireAssertion("run_success", report.success);
requireAssertion("follow_up_matched", report.followUp.matched !== false);
requireAssertion("follow_up_semantic_shape", !followUpPattern || followUpPattern.test(String(report.followUp.assistant_text || "").trim()));
requireAssertion("follow_up_forbidden_content_absent", !followUpForbiddenParts.some(part => String(report.followUp.assistant_text || "").includes(part)));
requireAssertion("follow_up_tool_budget", followUpMaximumTools < 0 || Number(report.followUp.tool_calls || 0) <= followUpMaximumTools);
requireAssertion("exact_answer", !expectedExact || report.assistantText.trim() === expectedExact);
requireAssertion("required_answer_content", expectedParts.every(part => report.assistantText.includes(part)));
requireAssertion("forbidden_answer_content_absent", !forbiddenParts.some(part => report.assistantText.includes(part)));
requireAssertion("markdown_headings_well_formed", !/^#{1,6}\s+[^\n]*#{1,6}\s+/m.test(report.assistantText));
requireAssertion("required_tool_completed", !requireTool || report.toolFinished >= 1);
requireAssertion("tool_error_budget", report.toolErrors <= maximumToolErrors);
requireAssertion("completed_tool_budget", maximumCompletedTools < 0 || report.toolFinished <= maximumCompletedTools);
requireAssertion("expected_tools_completed", expectedTools.every(tool => completedTools.has(tool)));
requireAssertion("tool_lifecycle_balanced", ![...startedCallIDs].some(callID => !finishedCallIDs.has(callID)));
requireAssertion("thinking_delta_budget", maximumThinkingDeltas <= 0 || report.thinkingDeltas <= maximumThinkingDeltas);
requireAssertion("incremental_body_mutations", report.bodyMutations >= minimumAssistantDeltas);
requireAssertion("single_terminal_event", report.finished === 1);
requireAssertion("assistant_reset_budget", report.assistantResets <= maximumAssistantResets);
requireAssertion("permission_request_count", expectedPermissionRequests < 0 || report.permissionRequests === expectedPermissionRequests);
requireAssertion("permission_request_minimum", report.permissionRequests >= minimumPermissionRequests);
requireAssertion("permission_requests_reached_ui", report.permissionCardsPresented === report.permissionRequests);
requireAssertion("render_probe_matches_assistant", renderProbe.canonical_text === report.assistantText);
requireAssertion("stable_committed_prefix", renderProbe.stable_committed_prefix === true);
requireAssertion("render_nodes_committed", renderProbe.committed_node_count >= 1);
requireAssertion("internal_control_tools_hidden", renderProbe.internal_control_tool_visible === false);
requireAssertion("visible_tool_count", renderProbe.visible_tool_count >= minimumVisibleTools);
requireAssertion("markdown_node_count", renderProbe.markdown_node_count >= minimumMarkdownNodes);
requireAssertion("native_table_structure", Number(renderProbe.rendered_table_count || 0) >= minimumRenderedTables);
requireAssertion("code_node_count", renderProbe.code_node_count >= minimumCodeNodes);
requireAssertion("mutable_tail_updates", Number(renderProbe.mutable_tail_update_count || 0) >= minimumMutableTailUpdates);
requireAssertion("requested_screenshot", !screenshotRequested || (
  screenshot.captured === true && screenshot.bytes >= 10000 && fs.existsSync(process.argv[8])
));
requireAssertion("embedded_host_terminated", !report.hostAlive);
report.failedRuntimeAssertions = failedAssertions;
console.log(JSON.stringify(report));
if (failedAssertions.length > 0) process.exit(9);
NODE
    RUNTIME_ACCEPTANCE_FAILED=1
fi

ARTIFACT_ACCEPTANCE_FAILED=0
if [[ -n "${NOLA_MAC_REAL_EXPECTED_ARTIFACT:-}" ]]; then
    ARTIFACT="$WORKSPACE/$NOLA_MAC_REAL_EXPECTED_ARTIFACT"
    VALIDATOR_ARGS=(
        "$ARTIFACT"
        --contains-json "${NOLA_MAC_REAL_ARTIFACT_CONTAINS_JSON:-[]}"
        --quick-look
        --report "$ARTIFACT_ROOT/final-artifact-acceptance.json"
    )
    if [[ -n "${NOLA_MAC_REAL_ARTIFACT_KIND:-}" ]]; then
        VALIDATOR_ARGS+=(--kind "$NOLA_MAC_REAL_ARTIFACT_KIND")
    fi
    if [[ -n "${NOLA_MAC_REAL_EXPECTED_TABLES:-}" ]]; then
        VALIDATOR_ARGS+=(--expected-tables "$NOLA_MAC_REAL_EXPECTED_TABLES")
    fi
    if [[ -n "${NOLA_MAC_REAL_EXPECTED_ROWS:-}" ]]; then
        VALIDATOR_ARGS+=(--expected-rows "$NOLA_MAC_REAL_EXPECTED_ROWS")
    fi
    if [[ -n "${NOLA_MAC_REAL_EXPECTED_COLUMNS:-}" ]]; then
        VALIDATOR_ARGS+=(--expected-columns "$NOLA_MAC_REAL_EXPECTED_COLUMNS")
    fi
    if [[ "${NOLA_MAC_REAL_VALIDATE_WITH_WORD:-0}" == "1" ]]; then
        VALIDATOR_ARGS+=(--microsoft-word)
    fi
    printf '[real-mac-agent] validating final artifact and external renderer\n'
    if ! python3 "$ROOT/scripts/validate_final_artifact.py" "${VALIDATOR_ARGS[@]}"; then
        ARTIFACT_ACCEPTANCE_FAILED=1
    fi
fi

PERSISTENCE_ACCEPTANCE_FAILED=0
SESSION_AUDIT="$ARTIFACT_ROOT/runtime-session-audit.json"
printf '[real-mac-agent] auditing durable production state\n'
if ! python3 "$ROOT/scripts/audit_runtime_session.py" \
    --db "$RUNTIME_DB" \
    --session-id "$SESSION_ID" \
    --presentation-receipt "$RESULT" \
    --expected-result "${NOLA_MAC_REAL_EXPECTED_RESULT:-success}" \
    --provider-trace "$PROVIDER_TRACE" \
    --canonical-reader "$CANONICAL_READER" \
    --receipt "$SESSION_AUDIT" \
    --allow-batched-message-timestamps; then
    PERSISTENCE_ACCEPTANCE_FAILED=1
fi

PERMISSION_FIXTURE_ACCEPTANCE_FAILED=0
if [[ -n "${NOLA_MAC_REAL_PERMISSION_FIXTURE:-}" ]]; then
    PERMISSION_DECISION=allow_once
    if [[ "${NOLA_MAC_REAL_PERMISSION_DECISION:-allow}" == "deny" ]]; then
        PERMISSION_DECISION=deny
    fi
    if ! python3 "$ROOT/scripts/audit_mac_permission_fixture.py" --workspace "$WORKSPACE" \
        --decision "$PERMISSION_DECISION" \
        --original "${NOLA_MAC_REAL_PERMISSION_ORIGINAL:?permission fixture original bytes required}" \
        --expected "${NOLA_MAC_REAL_PERMISSION_EXPECTED:?permission fixture expected bytes required}"; then
        PERMISSION_FIXTURE_ACCEPTANCE_FAILED=1
    fi
fi

SEMANTIC_ACCEPTANCE_FAILED=0
AUTOMATION_ACCEPTANCE_FAILED=0
if [[ "${NOLA_MAC_RUNTIME_ACCEPTANCE_AUTOMATION_CHAIN:-0}" == "1" ]]; then
    if ! python3 "$ROOT/scripts/verify_mac_automation_acceptance.py" "$RESULT" \
        --receipt "$ARTIFACT_ROOT/automation-chain-audit.json"; then
        AUTOMATION_ACCEPTANCE_FAILED=1
    fi
fi
SEMANTIC_AUDIT="$ARTIFACT_ROOT/semantic-response-audit.json"
if [[ "${NOLA_MAC_REAL_JUDGE:-0}" == "1" ]]; then
    printf '[real-mac-agent] judging answer against real tool evidence\n'
    if ! python3 "$ROOT/scripts/judge_real_e2e_response.py" \
        --config "$CONFIG_PATH" \
        --db "$RUNTIME_DB" \
        --session-id "$SESSION_ID" \
        --task "$PROMPT" \
        --current-date "$(date +%F)" \
        --receipt "$SEMANTIC_AUDIT"; then
        SEMANTIC_ACCEPTANCE_FAILED=1
    fi
fi

python3 - "$ROOT" "$RESULT" "$SCREENSHOT" "$APP_SHA256" "$APP_CODE_SHA256" "$HOST_SHA256" "$RUNTIME_DB" "$PROVIDER_TRACE" <<'PY'
import json
from pathlib import Path
import sys

root, result_path, screenshot_path, app_sha256, app_code_sha256, host_sha256, runtime_db, provider_trace = sys.argv[1:]
sys.path.insert(0, str(Path(root) / "scripts"))
from real_e2e_evidence import record_step
import os

value = json.loads(Path(result_path).read_text(encoding="utf-8"))
events = value.get("events", [])
count = lambda kind: sum(event.get("kind") == kind for event in events)
probe = value.get("render_probe", {})
follow_up = value.get("follow_up", {})
session_audit = str(Path(result_path).with_name("runtime-session-audit.json"))
artifacts = [result_path, runtime_db, f"{result_path}.events.jsonl", session_audit]
automation_audit = Path(result_path).with_name("automation-chain-audit.json")
if automation_audit.is_file():
    artifacts.append(str(automation_audit))
semantic_audit = Path(result_path).with_name("semantic-response-audit.json")
if semantic_audit.is_file():
    artifacts.append(str(semantic_audit))
permission_audit = Path(result_path).with_name("permission-fixture-audit.json")
if permission_audit.is_file():
    artifacts.extend([str(permission_audit), f"{result_path}.permission-audit.jsonl"])
if screenshot_path:
    artifacts.append(screenshot_path)
assertions = {
    "run_succeeded": value.get("success") is True,
    "tools_completed": os.environ.get("NOLA_MAC_REAL_REQUIRE_TOOL", "1") != "1"
        or count("toolFinished") >= 1,
    "tool_budget_respected": int(os.environ.get("NOLA_MAC_REAL_MAX_COMPLETED_TOOLS", "-1")) < 0
        or count("toolFinished") <= int(os.environ["NOLA_MAC_REAL_MAX_COMPLETED_TOOLS"]),
    "single_terminal": count("finished") == 1,
    "stable_stream_prefix": probe.get("stable_committed_prefix") is True,
    "authoritative_body_projected": int(probe.get("body_mutation_count", 0)) >= int(
        os.environ.get("NOLA_MAC_REAL_MIN_ASSISTANT_DELTAS", "1")
    ),
    "stream_tail_progressive": int(probe.get("mutable_tail_update_count", 0)) >= int(
        os.environ.get("NOLA_MAC_REAL_MIN_MUTABLE_TAIL_UPDATES", "0")
    ),
    "rendered_markdown": int(probe.get("markdown_node_count", 0)) >= int(
        os.environ.get("NOLA_MAC_REAL_MIN_MARKDOWN_NODES", "0")
    ),
    "follow_up_exact": not follow_up or follow_up.get("matched") is True,
    "follow_up_has_no_tools": not follow_up or int(follow_up.get("tool_calls", -1)) == 0,
    "isolated_database": Path(runtime_db).is_file(),
    "durable_session_audited": Path(session_audit).is_file()
        and json.loads(Path(session_audit).read_text(encoding="utf-8")).get("passed") is True,
    "semantic_answer_audited": os.environ.get("NOLA_MAC_REAL_JUDGE", "0") != "1"
        or semantic_audit.is_file()
        and json.loads(semantic_audit.read_text(encoding="utf-8")).get("passed") is True,
}
if os.environ.get("NOLA_MAC_RUNTIME_ACCEPTANCE_AUTOMATION_CHAIN", "0") == "1":
    assertions["automation_exact_occurrence_sealed"] = (
        automation_audit.is_file()
        and json.loads(automation_audit.read_text(encoding="utf-8")).get("passed") is True
    )
if os.environ.get("NOLA_MAC_REAL_PERMISSION_FIXTURE"):
    permission_verified = permission_audit.is_file() and json.loads(permission_audit.read_text()).get("passed") is True
    assertions.update({key: permission_verified for key in (
        "exact_pending_permission_audited", "original_fixture_intact_before_decision",
        "permission_result_sealed", "permission_execution_count_verified", "permission_final_bytes_verified")})
if os.environ.get("NOLA_MAC_REAL_REQUIRE_MULTITOOL_CONTRACT") == "1":
    started = [event for event in events if event.get("kind") == "toolStarted"]
    finished = [event for event in events if event.get("kind") == "toolFinished"]
    assertions.update({
        "tool_scope_covered": len(finished) >= 2,
        "every_tool_has_result": len(finished) >= len(started),
        "no_protocol_errors": not any(event.get("kind") == "failed" for event in events),
    })
record_step(
    "native-permission-overwrite" if os.environ.get("NOLA_MAC_REAL_PERMISSION_FIXTURE") else "native-chat-multitool-render-follow-up",
    platforms=["mac"],
    modules=["chat", "agent", "core", "provider", "tools", "persistence", "rendering", "recovery"]
        + (["permissions"] if os.environ.get("NOLA_MAC_REAL_PERMISSION_FIXTURE") else []),
    boundaries=["mac-product-runtime", "agent-host", "provider", "persistence"]
        + (["permission-runtime"] if os.environ.get("NOLA_MAC_REAL_PERMISSION_FIXTURE") else []),
    assertions=assertions,
    assertion_evidence={
        assertion_id: (
            ["mac-permission-audit"] if assertion_id in {
                "exact_pending_permission_audited", "original_fixture_intact_before_decision",
                "permission_result_sealed", "permission_execution_count_verified", "permission_final_bytes_verified"} else
            ["mac-automation-audit"] if assertion_id == "automation_exact_occurrence_sealed" else
            ["mac-events", "mac-runtime-result"]
            if assertion_id not in {"isolated_database", "durable_session_audited", "semantic_answer_audited"}
            else ["mac-runtime-db"]
            if assertion_id == "isolated_database"
            else ["mac-session-audit"]
            if assertion_id == "durable_session_audited"
            else ["mac-semantic-audit"] if semantic_audit.is_file() else ["mac-runtime-result"]
        )
        for assertion_id in assertions
    },
    observations=[
        {"id": "mac-provider-traffic", "kind": "provider_trace", "path": provider_trace},
        {"id": "mac-events", "kind": "event_stream", "path": f"{result_path}.events.jsonl"},
        {"id": "mac-runtime-result", "kind": "runtime_receipt", "path": result_path},
        {"id": "mac-runtime-db", "kind": "persistence_store", "path": runtime_db},
        {"id": "mac-session-audit", "kind": "persistence_audit", "path": session_audit},
    ] + ([
        {"id": "mac-permission-audit", "kind": "persistence_audit", "path": str(permission_audit)},
    ] if permission_audit.is_file() else []) + ([
        {"id": "mac-automation-audit", "kind": "persistence_audit", "path": str(automation_audit)},
    ] if automation_audit.is_file() else []) + ([
        {"id": "mac-semantic-audit", "kind": "semantic_audit", "path": str(semantic_audit)},
    ] if semantic_audit.is_file() else []) + ([
        {"id": "mac-render-screenshot", "kind": "ui_screenshot", "path": screenshot_path},
    ] if screenshot_path else []),
    metrics={
        "toolStarted": count("toolStarted"),
        "toolFinished": count("toolFinished"),
        "assistantDeltas": count("assistantDelta"),
        "visibleTools": probe.get("visible_tool_count", 0),
        "appSHA256": app_sha256,
        "appCodeSHA256": app_code_sha256,
        "embeddedHostSHA256": host_sha256,
    },
    artifacts=artifacts,
)
PY

if [[ "$RUNTIME_ACCEPTANCE_FAILED" == "1" \
    || "$ARTIFACT_ACCEPTANCE_FAILED" == "1" \
    || "$PERSISTENCE_ACCEPTANCE_FAILED" == "1" \
    || "$PERMISSION_FIXTURE_ACCEPTANCE_FAILED" == "1" \
    || "$AUTOMATION_ACCEPTANCE_FAILED" == "1" \
    || "$SEMANTIC_ACCEPTANCE_FAILED" == "1" ]]; then
    printf '[real-mac-agent] failed: runtime=%s artifact=%s persistence=%s semantic=%s automation=%s permission=%s\n' \
        "$RUNTIME_ACCEPTANCE_FAILED" "$ARTIFACT_ACCEPTANCE_FAILED" \
        "$PERSISTENCE_ACCEPTANCE_FAILED" "$SEMANTIC_ACCEPTANCE_FAILED" "$AUTOMATION_ACCEPTANCE_FAILED" \
        "$PERMISSION_FIXTURE_ACCEPTANCE_FAILED" >&2
    exit 9
fi

printf '[real-mac-agent] passed\n'
