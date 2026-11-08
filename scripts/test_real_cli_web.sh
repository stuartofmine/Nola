#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Real-provider acceptance for the CLI and the CLI-hosted Web product paths.
# The clients read ~/.nola/config.json themselves; this script never prints secrets.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CONFIG_PATH="${NOLA_CONFIG_PATH:-$HOME/.nola/config.json}"
BINARY="${NOLA_REAL_CLI_BINARY:-$ROOT/target/release/nola}"

[[ -f "$CONFIG_PATH" ]] || {
    printf 'Missing Nola config: %s\n' "$CONFIG_PATH" >&2
    exit 2
}

python3 - "$CONFIG_PATH" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    llm = json.load(handle).get("llm", {})
missing = [
    key
    for key in ("endpoint", "api_key", "model")
    if not str(llm.get(key, "")).strip()
]
if missing:
    raise SystemExit(
        "Nola config must define llm.endpoint, llm.api_key, and llm.model."
    )
PY

if [[ "${NOLA_SKIP_CLI_BUILD:-0}" != "1" ]]; then
    printf '[real-cli-web] building current CLI Web Host\n'
    cargo build --release --bin nola --manifest-path "$ROOT/cli/Cargo.toml"
    cargo build --release --bin nola-service --manifest-path "$ROOT/core/nola-service/Cargo.toml"
fi

[[ -x "$BINARY" ]] || {
    printf 'Nola CLI executable is missing: %s\n' "$BINARY" >&2
    exit 2
}
[[ -x "$(dirname "$BINARY")/nola-service" ]] || {
    printf 'Nola service executable is missing beside the CLI: %s\n' "$(dirname "$BINARY")/nola-service" >&2
    exit 2
}

if [[ "${NOLA_REAL_WEB_ONLY:-0}" != "1" ]]; then
printf '[real-cli-web] running CLI product path\n'
python3 - "$BINARY" "$CONFIG_PATH" <<'PY'
import json
import os
import shutil
import subprocess
import sys
import tempfile

binary, config_path = sys.argv[1:]
expected = "NOLA_CLI_REAL_OK"
with tempfile.TemporaryDirectory(prefix="nola-real-cli-") as runtime_dir:
    runtime_home = os.path.join(runtime_dir, "home")
    runtime_config_dir = os.path.join(runtime_home, ".nola")
    os.makedirs(runtime_config_dir, exist_ok=True)
    shutil.copyfile(config_path, os.path.join(runtime_config_dir, "config.json"))
    environment = os.environ.copy()
    environment["HOME"] = runtime_home
    environment["NOLA_DB_PATH"] = os.path.join(runtime_dir, "nola.sqlite")
    completed = subprocess.run(
        [
            binary,
            "--no-save",
            "--no-tools",
            "--json",
            f"Return only this exact ASCII token: {expected}",
        ],
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        timeout=90,
        env=environment,
    )
events = []
for line in completed.stdout.splitlines():
    try:
        events.append(json.loads(line))
    except json.JSONDecodeError:
        continue
assistant_text = ""
canonical_delta_count = 0
canonical_complete_count = 0
for event in events:
    if event.get("type") == "core_part_updated":
        part = event.get("part") or {}
        update = event.get("update") or {}
        if part.get("kind") == "text":
            assistant_text = str(part.get("text", ""))
            if update.get("type") == "text_delta":
                canonical_delta_count += 1
            if update.get("type") == "completed":
                canonical_complete_count += 1
types = [event.get("type") for event in events]
report = {
    "exitCode": completed.returncode,
    "eventCount": len(events),
    "assistantDeltaCount": canonical_delta_count,
    "assistantResetCount": types.count("assistant_reset"),
    "thinkingDeltaCount": types.count("thinking_delta"),
    "assistantCompleteCount": canonical_complete_count,
    "assistantText": assistant_text,
}
print(json.dumps(report, ensure_ascii=False))
if (
    completed.returncode != 0
    or report["assistantDeltaCount"] < 1
    or report["assistantResetCount"] != 0
    or report["assistantCompleteCount"] != 1
    or assistant_text.strip() != expected
):
    if completed.stderr.strip():
        print(completed.stderr, file=sys.stderr)
    raise SystemExit(9)
PY

printf '[real-cli-web] running interactive terminal product path\n'
env -u NOLA_E2E_RECEIPT_PATH \
    python3 "$ROOT/scripts/test_real_cli_terminal.py" "$BINARY"

printf '[real-cli-web] running Agent tools and compaction product path\n'
python3 "$ROOT/scripts/test_real_cli_agent.py" "$BINARY"
fi

printf '[real-cli-web] running token-session Web product path\n'
python3 - "$ROOT" "$BINARY" "$CONFIG_PATH" <<'PY'
import http.cookiejar
import json
import os
import shutil
import signal
import time
import socket
import subprocess
import sys
import tempfile
import time
import urllib.request
import uuid
from pathlib import Path

root, binary, config_path = sys.argv[1:]
expected = "NOLA_WEB_REAL_OK"
prompt = f"Return only this exact ASCII token: {expected}"
runtime_dir = tempfile.mkdtemp(prefix="nola-real-web-", dir="/tmp")

with socket.socket() as listener:
    listener.bind(("127.0.0.1", 0))
    port = listener.getsockname()[1]

environment = os.environ.copy()
runtime_home = os.path.join(runtime_dir, "home")
runtime_config_dir = os.path.join(runtime_home, ".nola")
os.makedirs(runtime_config_dir, exist_ok=True)
shutil.copyfile(config_path, os.path.join(runtime_config_dir, "config.json"))
with open(os.path.join(runtime_config_dir, "config.json"), encoding="utf-8") as handle:
    runtime_config = json.load(handle)
upstream_endpoint = str(runtime_config.get("llm", {}).get("endpoint", "")).strip()
if not upstream_endpoint:
    raise RuntimeError("configured provider endpoint is missing")
provider_trace_path = os.path.join(runtime_dir, "provider.ndjson")
provider_ready_path = os.path.join(runtime_dir, "provider-ready.json")
provider_log_path = os.path.join(runtime_dir, "provider-probe.log")
provider_log = open(provider_log_path, "w", encoding="utf-8")
provider_process = subprocess.Popen(
    [
        sys.executable,
        os.path.join(root, "scripts", "e2e", "provider_probe.py"),
        "--upstream", upstream_endpoint,
        "--log", provider_trace_path,
        "--ready", provider_ready_path,
    ],
    stdout=provider_log,
    stderr=subprocess.STDOUT,
    text=True,
)
deadline = time.monotonic() + 10
while not os.path.isfile(provider_ready_path):
    if provider_process.poll() is not None:
        provider_log.close()
        raise RuntimeError("provider evidence probe exited before readiness")
    if time.monotonic() >= deadline:
        raise RuntimeError("provider evidence probe readiness timed out")
    time.sleep(0.05)
with open(provider_ready_path, encoding="utf-8") as handle:
    runtime_config.setdefault("llm", {})["endpoint"] = json.load(handle)["endpoint"]
with open(os.path.join(runtime_config_dir, "config.json"), "w", encoding="utf-8") as handle:
    json.dump(runtime_config, handle)
environment["HOME"] = runtime_home
environment["NOLA_DB_PATH"] = os.path.join(runtime_dir, "nola.sqlite")
server_log_path = os.path.join(runtime_dir, "web-host.log")
server_log = open(server_log_path, "w", encoding="utf-8")
process = subprocess.Popen(
    [
        binary,
        "web",
        "--host",
        "127.0.0.1",
        "--port",
        str(port),
        "--web-root",
        os.path.join(root, "web"),
    ],
    # The Web Host's process workspace is intentionally isolated from the
    # repository. Absolute web/config paths still exercise the production
    # binary while relative model-generated artifacts cannot pollute source.
    cwd=runtime_dir,
    env=environment,
    stdout=server_log,
    stderr=subprocess.STDOUT,
    text=True,
)
base_url = f"http://127.0.0.1:{port}"

try:
    deadline = time.monotonic() + 15
    while True:
        if process.poll() is not None:
            raise RuntimeError("nola web exited before readiness")
        try:
            urllib.request.urlopen(f"{base_url}/login", timeout=0.5).read(1)
            break
        except Exception:
            if time.monotonic() >= deadline:
                raise RuntimeError("nola web readiness timed out")
            time.sleep(0.1)

    token_path = os.path.join(
        runtime_home, ".nola", "web", "mobile-access-token"
    )
    deadline = time.monotonic() + 5
    while not os.path.isfile(token_path):
        if time.monotonic() >= deadline:
            raise RuntimeError("Web Host token was not created")
        time.sleep(0.05)
    with open(token_path, encoding="utf-8") as handle:
        token = handle.read().strip()

    cookie_jar = http.cookiejar.CookieJar()
    opener = urllib.request.build_opener(
        urllib.request.HTTPCookieProcessor(cookie_jar)
    )

    def json_request(path, body=None, *, method=None):
        data = None if body is None else json.dumps(body).encode()
        headers = {} if data is None else {"Content-Type": "application/json"}
        request = urllib.request.Request(
            f"{base_url}{path}",
            data=data,
            headers=headers,
            method=method or ("GET" if data is None else "POST"),
        )
        try:
            with opener.open(request, timeout=10) as response:
                return json.load(response)
        except urllib.error.HTTPError as error:
            payload = error.read().decode("utf-8", "replace")
            raise RuntimeError(
                f"HTTP {error.code} for {path}: {payload}"
            ) from error

    def canonical_identity():
        submission_id = str(uuid.uuid4())
        return {
            "submission_id": submission_id,
            "user_message_id": f"{submission_id}:user",
            "assistant_message_id": f"{submission_id}:assistant",
        }

    def stream_agent(
        conversation_id,
        prompt_text,
        client_id,
        *,
        agent_mode,
        permission_decision=None,
    ):
        request = urllib.request.Request(
            f"{base_url}/api/chat/agent/v2/stream",
            data=json.dumps(
                {
                    "conversationId": conversation_id,
                    **canonical_identity(),
                    "client_id": client_id,
                    "messages": [{"role": "user", "content": prompt_text}],
                    "agentMode": agent_mode,
                    "language": "en",
                    "responseStyle": "concise",
                    "max_iterations": 12,
                }
            ).encode(),
            headers={
                "Content-Type": "application/json",
                "Accept": "text/event-stream",
            },
            method="POST",
        )
        stream_events = []
        with opener.open(request, timeout=180) as response:
            for raw_line in response:
                line = raw_line.decode("utf-8", "replace").strip()
                if not line.startswith("data:"):
                    continue
                payload = line[5:].strip()
                if not payload or payload == "[DONE]":
                    continue
                event = json.loads(payload)
                stream_events.append(event)
                if event.get("type") != "permission_request":
                    continue
                if permission_decision not in {"allow", "deny"}:
                    raise RuntimeError(
                        "unexpected Web permission request: "
                        + json.dumps(event, ensure_ascii=False)
                    )
                json_request(
                    "/api/agent/host-permission/decision",
                    {
                        "protocol_version": event.get("protocol_version", 0),
                        "nonce": event.get("nonce", ""),
                        "request_id": event.get("request_id") or event.get("id", ""),
                        "host_id": event.get("host_id", ""),
                        "client_id": event.get("client_id", ""),
                        "session_id": event.get("session_id", ""),
                        "tool_name": event.get("tool_name") or event.get("name", ""),
                        "args_digest": event.get("args_digest", ""),
                        "decision": permission_decision,
                    },
                )
        return stream_events

    def canonical_trace(event):
        for key in ("trace", "event"):
            candidate = event.get(key)
            if isinstance(candidate, dict) and candidate.get("kind"):
                return candidate
        return {}

    def assistant_projection(events):
        ordered_part_ids = []
        text_parts = {}
        canonical_updates = 0
        for event in events:
            if event.get("type") != "core_part_updated":
                continue
            part = event.get("part") if isinstance(event.get("part"), dict) else {}
            if (part.get("kind") or part.get("type")) != "text":
                continue
            part_id = str(event.get("part_id") or part.get("id") or "")
            if not part_id:
                part_id = f"text-part-{len(ordered_part_ids)}"
            if part_id not in text_parts:
                ordered_part_ids.append(part_id)
            text_parts[part_id] = str(part.get("text", ""))
            canonical_updates += 1
        if ordered_part_ids:
            return "".join(text_parts[part_id] for part_id in ordered_part_ids), canonical_updates
        return "", 0

    def assistant_from(events):
        text, _ = assistant_projection(events)
        return text.strip().strip("`").strip()

    def tool_event_identity(event):
        trace = canonical_trace(event)
        payload = trace.get("payload") if isinstance(trace.get("payload"), dict) else {}
        kind = trace.get("kind") or event.get("type") or ""
        name = (
            payload.get("tool_name")
            or event.get("tool_name")
            or event.get("name")
            or event.get("tool")
            or ""
        )
        return kind, name

    def tool_started_count(events, tool_name):
        return sum(
            kind in {"tool.started", "tool_call", "tool_started"}
            and name == tool_name
            for kind, name in map(tool_event_identity, events)
        )

    def tool_lifecycle_count(events, tool_name):
        return sum(
            kind
            in {
                "tool.started",
                "tool.completed",
                "tool.failed",
                "tool_call",
                "tool_started",
                "tool_result",
                "tool_finished",
            }
            and name == tool_name
            for kind, name in map(tool_event_identity, events)
        )

    login = json_request("/api/web/login", {"token": token})
    session = json_request("/api/web/session")
    json_request(
        "/api/agent/settings",
        {
            "agent_mode_enabled": True,
            "host_tools_enabled": True,
            "enable_local_dev_tools": True,
            "network_policy": "allow",
            "allowed_dirs": [runtime_dir],
        },
    )
    conversation = json_request(
        "/api/chat/conversations",
        {
            "title": "Real Web acceptance",
            "selectedModelSource": "auto",
            "selectedModelId": "",
        },
    )
    conversation_id = (
        conversation.get("conversationId")
        or conversation.get("conversation_id")
        or conversation.get("id")
    )
    goal_start = json_request(
        f"/api/chat/conversations/{conversation_id}/goal",
        {
            "action": "start",
            "objective": "Verify durable Web Goal lifecycle",
            "constraints": ["Persist every transition through Core"],
        },
    )
    goal_pause = json_request(
        f"/api/chat/conversations/{conversation_id}/goal",
        {"action": "pause"},
    )
    goal_schedule = json_request(
        f"/api/chat/conversations/{conversation_id}/goal",
        {"action": "schedule", "wakeAtEpochMs": int(time.time() * 1000) + 60_000},
    )
    goal_resume = json_request(
        f"/api/chat/conversations/{conversation_id}/goal",
        {"action": "resume"},
    )
    goal_cancel = json_request(
        f"/api/chat/conversations/{conversation_id}/goal",
        {"action": "cancel"},
    )
    goal_readback = json_request(
        f"/api/chat/conversations/{conversation_id}/goal"
    )

    data_source_code = f"WEB-DATA-SOURCE-{time.time_ns()}"
    data_source_path = os.path.join(runtime_dir, "verified-context.csv")
    with open(data_source_path, "w", encoding="utf-8") as handle:
        handle.write(f"project_code,owner\n{data_source_code},Nola Web E2E\n")
    created_source = json_request(
        "/api/data-sources",
        {
            "name": "Verified Web acceptance context",
            "kind": "localFile",
            "location": data_source_path,
            "accessPolicy": "conversationOptIn",
            "maximumAgeMinutes": 60,
        },
    )
    source = created_source.get("source", {})
    source_id = source.get("id", "")
    source_contract = source.get("validationContract", {})
    enabled_source = json_request(
        f"/api/chat/conversations/{conversation_id}/data-sources/{source_id}",
        {"enabled": True},
        method="PATCH",
    )
    data_source_prompt = (
        "Read the verified connected data source for this conversation. "
        "Return only its exact project_code value and nothing else."
    )
    data_source_events = stream_agent(
        conversation_id,
        data_source_prompt,
        "real-web-data-source-acceptance",
        agent_mode=False,
    )
    data_source_answer = assistant_from(data_source_events)
    data_source_usage = json_request("/api/data-sources/usage").get("usageRecords", [])
    stream_request = urllib.request.Request(
        f"{base_url}/api/chat/agent/v2/stream",
        data=json.dumps(
            {
                "conversationId": conversation_id,
                **canonical_identity(),
                "client_id": "real-web-acceptance",
                "messages": [{"role": "user", "content": prompt}],
                "agentMode": False,
                "language": "en",
                "responseStyle": "concise",
                "max_iterations": 4,
            }
        ).encode(),
        headers={
            "Content-Type": "application/json",
            "Accept": "text/event-stream",
        },
        method="POST",
    )
    events = []
    stream_done = False
    with opener.open(stream_request, timeout=180) as response:
        for raw_line in response:
            line = raw_line.decode("utf-8", "replace").strip()
            if not line.startswith("data:"):
                continue
            payload = line[5:].strip()
            if payload == "[DONE]":
                stream_done = True
            elif payload:
                events.append(json.loads(payload))

    assistant_text, canonical_assistant_updates = assistant_projection(events)
    event_types = [event.get("type") for event in events]
    committed_projection_indices = [
        index for index, event in enumerate(events)
        if event.get("type") == "conversation_projection_committed_v1"
    ]
    terminal_trace_indices = [
        index for index, event in enumerate(events)
        if canonical_trace(event).get("kind")
        in {"run.completed", "run.failed", "run.cancelled"}
    ]
    committed_usage = next(
        (
            event.get("conversation_projection", {}).get("context_usage", {})
            for event in reversed(events)
            if event.get("type") == "conversation_projection_committed_v1"
        ),
        {},
    )
    if not assistant_text.strip():
        raise RuntimeError(
            "Web stream produced no projected assistant text: "
            + json.dumps(events[:12], ensure_ascii=False)
        )
    traces = json_request(
        f"/api/chat/conversations/{conversation_id}/traces"
    )
    messages = json_request(
        f"/api/chat/conversations/{conversation_id}/messages"
    ).get("messages", [])
    branch_boundary_id = messages[-1].get("messageId", "") if messages else ""
    branch_result = json_request(
        f"/api/chat/conversations/{conversation_id}/branch",
        {"messageId": branch_boundary_id, "title": "Real Web atomic branch"},
    )
    branch_conversation = branch_result.get("conversation", {})
    branch_conversation_id = (
        branch_conversation.get("conversationId")
        or branch_conversation.get("conversation_id")
        or branch_conversation.get("id")
        or ""
    )
    branch_messages = json_request(
        f"/api/chat/conversations/{branch_conversation_id}/messages"
    ).get("messages", []) if branch_conversation_id else []

    report = {
        "loginAuthenticated": login.get("authenticated") is True,
        "sessionAuthenticated": session.get("sessionAuthenticated") is True,
        "eventCount": len(events),
        "assistantDeltaCount": (
            canonical_assistant_updates
        ),
        "assistantResetCount": event_types.count("assistant_reset"),
        "thinkingDeltaCount": event_types.count("thinking_delta"),
        "streamCompleted": stream_done,
        "sessionProjectionCount": event_types.count("core_session_projection_updated"),
        "committedProjectionCount": len(committed_projection_indices),
        "commitFollowsTerminalTrace": bool(
            committed_projection_indices
            and terminal_trace_indices
            and committed_projection_indices[-1] > terminal_trace_indices[-1]
        ),
        "committedContextUsedTokens": int(committed_usage.get("used_tokens", 0) or 0),
        "committedContextWindowTokens": int(committed_usage.get("context_window_tokens", 0) or 0),
        "assistantText": assistant_text,
        "traceRuns": len(traces.get("traces", [])),
        "storedRoles": [message.get("role") for message in messages],
        "branchCreated": bool(branch_conversation_id),
        "branchHasDistinctIdentity": bool(branch_conversation_id) and branch_conversation_id != conversation_id,
        "branchStoredRoles": [message.get("role") for message in branch_messages],
        "branchMessageCount": len(branch_messages),
        "goalStarted": goal_start.get("goalProjection", {}).get("active", {}).get("status") == "running",
        "goalPaused": goal_pause.get("goalProjection", {}).get("active", {}).get("status") == "paused",
        "goalScheduled": goal_schedule.get("goalProjection", {}).get("active", {}).get("next_wake_at_epoch_ms") is not None,
        "goalResumed": goal_resume.get("goalProjection", {}).get("active", {}).get("status") == "running",
        "goalCancelledAndPersisted": goal_cancel.get("goalProjection", {}).get("active", {}).get("status") == "cancelled" and goal_readback.get("goalProjection", {}).get("active", {}).get("status") == "cancelled",
        "dataSourceCreatedAndTested": created_source.get("tested") is True,
        "dataSourceValidationState": source_contract.get("validationState"),
        "dataSourceRecordCount": source_contract.get("recordCount", 0),
        "dataSourceConversationEnabled": enabled_source.get("source", {}).get("resolution", {}).get("usable") is True,
        "dataSourceAnswer": data_source_answer,
        "dataSourceAnswerExact": data_source_answer == data_source_code,
        "dataSourceAnswerContainsUniqueCode": (
            data_source_answer.count(data_source_code) == 1
        ),
        "dataSourceUsageAudited": any(
            item.get("sourceId") == source_id
            and item.get("conversationId") == conversation_id
            and item.get("disclosedToCloud") is True
            for item in data_source_usage
        ),
    }

    write_path = os.path.join(runtime_dir, "web-recommended-write.txt")
    write_expected = "NOLA_WEB_WRITE_OK"
    write_prompt = (
        "Use the write_file tool to create the absolute file "
        f"{write_path} with exactly this content: WEB_RECOMMENDED_WRITE. "
        "Do not use bash. After success return only this exact ASCII token: "
        + write_expected
    )
    write_conversation = json_request(
        "/api/chat/conversations",
        {
            "title": "Real Web write permission acceptance",
            "selectedModelSource": "auto",
            "selectedModelId": "",
        },
    )
    write_conversation_id = (
        write_conversation.get("conversationId")
        or write_conversation.get("conversation_id")
        or write_conversation.get("id")
    )
    write_events = stream_agent(
        write_conversation_id,
        write_prompt,
        "real-web-write-acceptance",
        agent_mode=True,
    )
    write_text = assistant_from(write_events)
    write_file_text = ""
    if os.path.exists(write_path):
        with open(write_path, encoding="utf-8") as handle:
            write_file_text = handle.read()
    write_tool_traces = []
    for event in write_events:
        kind, name = tool_event_identity(event)
        if name != "write_file":
            continue
        trace = canonical_trace(event)
        write_tool_traces.append(
            {
                "kind": kind,
                "status": trace.get("status"),
                "payload": trace.get("payload"),
            }
        )

    protected_path = os.path.join(runtime_dir, "web-protected.txt")
    with open(protected_path, "w", encoding="utf-8") as handle:
        handle.write("KEEP_WEB_PROTECTED")
    delete_expected = "NOLA_WEB_DELETE_DENIED_OK"
    delete_prompt = (
        "Use the delete_file tool exactly once to delete the absolute file "
        f"{protected_path}. Do not use bash or another deletion tool. If permission "
        "is denied, do not retry and return only this exact ASCII token: "
        + delete_expected
    )
    delete_conversation = json_request(
        "/api/chat/conversations",
        {
            "title": "Real Web destructive permission acceptance",
            "selectedModelSource": "auto",
            "selectedModelId": "",
        },
    )
    delete_conversation_id = (
        delete_conversation.get("conversationId")
        or delete_conversation.get("conversation_id")
        or delete_conversation.get("id")
    )
    delete_events = stream_agent(
        delete_conversation_id,
        delete_prompt,
        "real-web-delete-acceptance",
        agent_mode=True,
        permission_decision="deny",
    )
    delete_text = assistant_from(delete_events)
    with open(protected_path, encoding="utf-8") as handle:
        protected_text = handle.read()
    report.update(
        {
            "writeAssistantText": write_text,
            "writePermissionRequests": sum(
                event.get("type") == "permission_request"
                for event in write_events
            ),
            "writeToolEvents": tool_lifecycle_count(write_events, "write_file"),
            "writeToolTraces": write_tool_traces,
            "writeFileText": write_file_text,
            "deleteAssistantText": delete_text,
            "deletePermissionRequests": sum(
                event.get("type") == "permission_request"
                for event in delete_events
            ),
            "deleteToolErrors": sum(
                event.get("type") in {"tool_result", "tool_finished"}
                and event.get("is_error") is True
                for event in delete_events
            ),
            "protectedFileText": protected_text,
        }
    )

    memory_code = f"WEB-PROJECT-LABEL-{time.time_ns()}"
    memory_write_conversation = json_request(
        "/api/chat/conversations",
        {
            "title": "Real Web memory write acceptance",
            "selectedModelSource": "auto",
            "selectedModelId": "",
        },
    )
    memory_write_id = (
        memory_write_conversation.get("conversationId")
        or memory_write_conversation.get("conversation_id")
        or memory_write_conversation.get("id")
    )
    memory_write_events = stream_agent(
        memory_write_id,
        (
            "Remember this across future conversations: my non-sensitive durable Web project label is "
            f"{memory_code}. Store that exact labeled fact in long-term memory. "
            "Do not use files. After memory is actually saved, return only "
            "MEMORY_WRITE_OK."
        ),
        "real-web-memory-write",
        agent_mode=True,
    )
    memory_recall_conversation = json_request(
        "/api/chat/conversations",
        {
            "title": "Real Web memory recall acceptance",
            "selectedModelSource": "auto",
            "selectedModelId": "",
        },
    )
    memory_recall_id = (
        memory_recall_conversation.get("conversationId")
        or memory_recall_conversation.get("conversation_id")
        or memory_recall_conversation.get("id")
    )
    memory_recall_events = stream_agent(
        memory_recall_id,
        (
            "This is a new conversation. Recall my durable Web project label from "
            "long-term memory. Do not infer, guess, or return labels for other "
            "platforms. Return only the exact saved project label."
        ),
        "real-web-memory-recall",
        agent_mode=True,
    )

    memory_write_started = tool_started_count(memory_write_events, "memory_save")
    memory_search_started = tool_started_count(memory_recall_events, "memory_search")
    memory_tool_traces = []
    for event in memory_write_events + memory_recall_events:
        trace = canonical_trace(event)
        kind, name = tool_event_identity(event)
        if name not in {"memory_save", "memory_search"}:
            continue
        memory_tool_traces.append(
            {
                "kind": kind,
                "name": name,
                "status": trace.get("status"),
                "payload": trace.get("payload"),
            }
        )
    report.update(
        {
            "memoryWriteAssistantText": assistant_from(memory_write_events),
            "memoryRecallAssistantText": assistant_from(memory_recall_events),
            "memoryRecallContainsExactLabel": (
                memory_code in assistant_from(memory_recall_events)
            ),
            "memoryRecallLabelCount": assistant_from(memory_recall_events).count(
                "WEB-PROJECT-LABEL-"
            ),
            "memoryRecallExactFormat": (
                assistant_from(memory_recall_events).strip() == memory_code
            ),
            "memorySaveStarted": memory_write_started,
            "memorySearchStarted": memory_search_started,
            "memorySaveCompleted": sum(
                item.get("kind") == "tool.completed"
                and item.get("name") == "memory_save"
                for item in memory_tool_traces
            ),
            "memorySearchCompleted": sum(
                item.get("kind") == "tool.completed"
                and item.get("name") == "memory_search"
                for item in memory_tool_traces
            ),
            "memoryToolFailures": sum(
                item.get("kind") == "tool.failed" for item in memory_tool_traces
            ),
            "memoryPermissionRequests": sum(
                event.get("type") == "permission_request"
                for event in memory_write_events + memory_recall_events
            ),
        }
    )
    if not report["memoryRecallContainsExactLabel"]:
        report["memoryFailureTraces"] = memory_tool_traces

    sales_path = os.path.join(runtime_dir, "web-skill-sales.csv")
    analysis_path = os.path.join(runtime_dir, "web-skill-analysis.md")
    with open(sales_path, "w", encoding="utf-8") as handle:
        handle.write(
            "month,region,revenue,cost\n"
            "Jan,North,120,80\n"
            "Jan,South,150,95\n"
            "Feb,North,140,88\n"
            "Feb,South,165,102\n"
        )
    skill_conversation = json_request(
        "/api/chat/conversations",
        {
            "title": "Real Web data-analysis Skill acceptance",
            "selectedModelSource": "auto",
            "selectedModelId": "",
        },
    )
    skill_conversation_id = (
        skill_conversation.get("conversationId")
        or skill_conversation.get("conversation_id")
        or skill_conversation.get("id")
    )
    skill_events = stream_agent(
        skill_conversation_id,
        (
            "Use the built-in analyze-data Skill to analyze the CSV at the "
            f"absolute path {sales_path}. Reconcile every total to the source and "
            f"write the validated Markdown report to exactly {analysis_path}. "
            "The report must explicitly discuss revenue and limitations. "
            "Do not merely describe what you would do."
        ),
        "real-web-data-analysis-skill",
        agent_mode=True,
        # The real model may clean up a temporary analysis script. That is a
        # destructive operation and must exercise the Web permission protocol
        # rather than making the fixture fail or weakening the product policy.
        permission_decision="allow",
    )
    skill_trace_sequences_by_run = {}
    for event in skill_events:
        trace = canonical_trace(event)
        sequence = trace.get("sequence")
        run_id = trace.get("run_id") or trace.get("trace_id")
        if sequence is None or not run_id:
            continue
        skill_trace_sequences_by_run.setdefault(str(run_id), []).append(int(sequence))
    skill_trace_ordered = bool(skill_trace_sequences_by_run) and all(
        sequences == sorted(set(sequences))
        for sequences in skill_trace_sequences_by_run.values()
    )
    skill_report_text = ""
    if os.path.isfile(analysis_path):
        with open(analysis_path, encoding="utf-8") as handle:
            skill_report_text = handle.read()
    report.update(
        {
            "dataSkillStarted": tool_started_count(skill_events, "Skill"),
            "dataSkillPermissionRequests": sum(
                event.get("type") == "permission_request" for event in skill_events
            ),
            "dataSkillRunCompleted": sum(
                canonical_trace(event).get("kind") == "run.completed"
                for event in skill_events
            ),
            "dataSkillTraceOrdered": skill_trace_ordered,
            "dataSkillArtifactExists": os.path.isfile(analysis_path),
            "dataSkillArtifactHasRevenue": "revenue" in skill_report_text.lower(),
            "dataSkillArtifactHasLimitations": "limitation" in skill_report_text.lower(),
        }
    )

    compact_conversation = json_request(
        "/api/chat/conversations",
        {
            "title": "Real Web compaction acceptance",
            "selectedModelSource": "auto",
            "selectedModelId": "",
        },
    )
    compact_conversation_id = (
        compact_conversation.get("conversationId")
        or compact_conversation.get("conversation_id")
        or compact_conversation.get("id")
    )
    for index in range(4):
        marker = f"HISTORICAL_TURN_{index}_MARKER"
        history_events = stream_agent(
            compact_conversation_id,
            f"Return exactly this ASCII token and nothing else: {marker}\n"
            + ("user-context " * 220),
            f"real-web-compaction-history-{index}",
            agent_mode=False,
        )
        if marker not in assistant_from(history_events):
            raise RuntimeError(f"compaction history turn {index} lost its marker")
    compact_result = json_request(
        f"/api/chat/conversations/{compact_conversation_id}/compact",
        {},
    )
    continuation_expected = "NOLA_WEB_COMPACT_CONTINUE_OK"
    continuation_prompt = (
        "Continue this same conversation and return only this exact ASCII token: "
        + continuation_expected
    )
    continuation_request = urllib.request.Request(
        f"{base_url}/api/chat/agent/v2/stream",
        data=json.dumps(
            {
                "conversationId": compact_conversation_id,
                **canonical_identity(),
                "client_id": "real-web-compaction-acceptance",
                "messages": [
                    {"role": "user", "content": continuation_prompt}
                ],
                "agentMode": False,
                "language": "en",
                "responseStyle": "concise",
                "max_iterations": 4,
            }
        ).encode(),
        headers={
            "Content-Type": "application/json",
            "Accept": "text/event-stream",
        },
        method="POST",
    )
    continuation_events = []
    with opener.open(continuation_request, timeout=180) as response:
        for raw_line in response:
            line = raw_line.decode("utf-8", "replace").strip()
            if not line.startswith("data:"):
                continue
            payload = line[5:].strip()
            if payload and payload != "[DONE]":
                continuation_events.append(json.loads(payload))
    continuation_text = assistant_from(continuation_events)
    continuation_event_types = [event.get("type") for event in continuation_events]
    compact_context = compact_result.get("context", {})
    report.update(
        {
            "compactReplacedItemCount": compact_context.get(
                "replacedItemCount", 0
            ),
            "compactRemainingItemCount": compact_context.get(
                "remainingItemCount", 0
            ),
            "compactContinuationText": continuation_text,
            "compactContinuationResetCount": sum(
                event.get("type") == "assistant_reset"
                for event in continuation_events
            ),
            "compactContinuationCommittedProjectionCount": continuation_event_types.count(
                "conversation_projection_committed_v1"
            ),
            "compactContinuationEventTypes": continuation_event_types,
            "compactSnapshotPreserved": compact_context.get("replacedItemCount", 0) > 0,
        }
    )
    print(json.dumps(report, ensure_ascii=False))
    if not all(
        [
            report["loginAuthenticated"],
            report["sessionAuthenticated"],
            report["assistantDeltaCount"] >= 1,
            report["assistantResetCount"] == 0,
            report["streamCompleted"],
            report["sessionProjectionCount"] >= 1,
            report["committedProjectionCount"] == 1,
            report["commitFollowsTerminalTrace"],
            report["committedContextUsedTokens"] > 0,
            report["committedContextWindowTokens"] > 0,
            assistant_text.strip() == expected,
            report["traceRuns"] >= 1,
            report["storedRoles"] == ["user", "assistant", "user", "assistant"],
            report["branchCreated"],
            report["branchHasDistinctIdentity"],
            report["branchStoredRoles"] == ["user", "assistant", "user", "assistant"],
            report["branchMessageCount"] == 4,
            report["goalStarted"],
            report["goalPaused"],
            report["goalScheduled"],
            report["goalResumed"],
            report["goalCancelledAndPersisted"],
            report["dataSourceCreatedAndTested"],
            report["dataSourceValidationState"] == "verified",
            report["dataSourceRecordCount"] >= 1,
            report["dataSourceConversationEnabled"],
            report["dataSourceAnswerContainsUniqueCode"],
            report["dataSourceUsageAudited"],
            report["writeAssistantText"].strip() == write_expected,
            report["writePermissionRequests"] == 0,
            report["writeToolEvents"] >= 2,
            report["writeFileText"] == "WEB_RECOMMENDED_WRITE",
            report["deleteAssistantText"].strip() == delete_expected,
            report["deletePermissionRequests"] == 1,
            report["protectedFileText"] == "KEEP_WEB_PROTECTED",
            report["memoryWriteAssistantText"] == "MEMORY_WRITE_OK",
            report["memoryRecallContainsExactLabel"],
            report["memoryRecallLabelCount"] == 1,
            report["memorySaveStarted"] == 1,
            report["memorySearchStarted"] in {1, 2, 3},
            report["memorySaveCompleted"] == 1,
            report["memorySearchCompleted"] >= 1,
            report["memoryToolFailures"] == 0,
            report["memoryPermissionRequests"] == 0,
            report["dataSkillStarted"] == 1,
            report["dataSkillPermissionRequests"] == 0,
            report["dataSkillRunCompleted"] == 1,
            report["dataSkillTraceOrdered"],
            report["dataSkillArtifactExists"],
            report["dataSkillArtifactHasRevenue"],
            report["dataSkillArtifactHasLimitations"],
            report["compactReplacedItemCount"] > 0,
            report["compactContinuationText"].strip()
            == continuation_expected,
            report["compactContinuationResetCount"] == 0,
            report["compactContinuationCommittedProjectionCount"] == 1,
            report["compactSnapshotPreserved"],
        ]
    ):
        raise SystemExit(9)
    receipt_path = os.environ.get("NOLA_E2E_RECEIPT_PATH", "").strip()
    if receipt_path:
        provider_process.send_signal(signal.SIGTERM)
        provider_process.wait(timeout=5)
        provider_process = None
        provider_log.close()
        evidence_dir = os.path.join(os.path.dirname(receipt_path), "web-runtime-evidence")
        os.makedirs(evidence_dir, exist_ok=True)
        report_path = os.path.join(evidence_dir, "web-runtime-report.json")
        audit_path = os.path.join(evidence_dir, "web-runtime-audit.json")
        trace_copy = os.path.join(evidence_dir, "provider.ndjson")
        database_copy = os.path.join(evidence_dir, "nola.sqlite")
        with open(report_path, "w", encoding="utf-8") as handle:
            json.dump(report, handle, ensure_ascii=False, indent=2)
        audit_assertions = {
            "permission_policy_verified": report["writePermissionRequests"] == 0 and report["deletePermissionRequests"] == 1 and report["protectedFileText"] == "KEEP_WEB_PROTECTED",
            "cross_session_memory_verified": report["memoryRecallExactFormat"] and report["memoryPermissionRequests"] == 0,
            "data_source_read_and_disclosure_audited": report["dataSourceAnswerExact"] and report["dataSourceUsageAudited"],
            "skill_artifact_verified": report["dataSkillArtifactExists"] and report["dataSkillArtifactHasRevenue"] and report["dataSkillArtifactHasLimitations"],
            "compaction_continuation_verified": report["compactReplacedItemCount"] > 0 and report["compactContinuationText"].strip() == continuation_expected,
        }
        with open(audit_path, "w", encoding="utf-8") as handle:
            json.dump({"passed": all(audit_assertions.values()), "assertions": audit_assertions}, handle, ensure_ascii=False, indent=2)
        shutil.copyfile(provider_trace_path, trace_copy)
        shutil.copyfile(environment["NOLA_DB_PATH"], database_copy)
        sys.path.insert(0, os.path.join(root, "scripts"))
        from real_e2e_evidence import record_step
        record_step(
            "web-integrated-agent-chain",
            platforms=["web"],
            modules=["agent", "audit", "chat", "compaction", "core", "data-source", "memory", "permissions", "persistence", "provider", "recovery", "skills", "tools", "ui"],
            boundaries=["cli-web-host", "http", "persistence", "provider", "sse"],
            assertions={"web_integrated_chain_passed": True},
            assertion_evidence={"web_integrated_chain_passed": ["provider-traffic", "web-runtime-report", "web-runtime-audit", "web-runtime-db"]},
            observations=[
                {"id": "provider-traffic", "kind": "provider_trace", "path": Path(trace_copy)},
                {"id": "web-runtime-report", "kind": "test_report", "path": Path(report_path)},
                {"id": "web-runtime-audit", "kind": "persistence_audit", "path": Path(audit_path)},
                {"id": "web-runtime-db", "kind": "persistence_store", "path": Path(database_copy)},
            ],
        )
finally:
    process.send_signal(signal.SIGTERM)
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=5)
    server_log.close()
    if provider_process is not None:
        provider_process.send_signal(signal.SIGTERM)
        try:
            provider_process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            provider_process.kill()
            provider_process.wait(timeout=5)
    if not provider_log.closed:
        provider_log.close()
    if os.environ.get("NOLA_REAL_WEB_PRESERVE_RUNTIME") == "1":
        print(json.dumps({"preservedRuntime": runtime_dir, "hostLog": server_log_path}))
    else:
        shutil.rmtree(runtime_dir, ignore_errors=True)
PY

printf '[real-cli-web] passed\n'
