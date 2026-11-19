#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Real-provider acceptance for the CLI and the CLI-hosted Web product paths.
# The clients read ~/.nola/config.json themselves; this script never prints secrets.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CONFIG_PATH="${NOLA_CONFIG_PATH:-$HOME/.nola/config.json}"
BINARY="${NOLA_REAL_CLI_BINARY:-$ROOT/target/release/nola}"

if (( $# > 1 )) || [[ $# -eq 1 && $1 != "--web-only" ]]; then
    printf 'Usage: %s [--web-only]\n' "$0" >&2
    exit 2
fi

[[ -f "$CONFIG_PATH" ]] || {
    printf 'Missing Nola config: %s\n' "$CONFIG_PATH" >&2
    exit 2
}

if [[ "${NOLA_SKIP_CLI_BUILD:-0}" != "1" ]]; then
    printf '[real-cli-web] building current CLI Web Host\n'
    cargo build --release --bin nola --manifest-path "$ROOT/apps/cli/Cargo.toml"
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

if [[ "${1:-}" != "--web-only" ]]; then
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
    environment.update(NOLA_HOME=runtime_home, NOLA_MAC_APP_SUPPORT=os.path.join(runtime_dir, "support"))
    config_file = os.path.join(runtime_config_dir, "config.json")
    with open(config_file) as source:
        if "connections" in json.load(source)["llm"]:
            subprocess.run([binary, "model", "apply-config", config_file], cwd=runtime_dir, env=environment,
                           check=True, stdout=subprocess.DEVNULL)
    completed = subprocess.run([binary, "--no-tools", "--json", f"Return only this exact ASCII token: {expected}"],
                               cwd=runtime_dir, env=environment, text=True, capture_output=True, timeout=120, check=True)
    events = [json.loads(line) for line in completed.stdout.splitlines() if line.startswith("{")]
    admitted, = [e for e in events if e.get("type") == "submission_admitted"]
    terminal, = [e for e in events if e.get("type") == "assistant_run_terminal_committed_v1"]
    assert (terminal["run_id"], terminal["session_id"]) == (admitted["run_id"], admitted["conversation_id"])
    shown = subprocess.check_output([binary, "--json", "session", "show", terminal["session_id"]],
                                    cwd=runtime_dir, env=environment)
    messages = json.loads(shown)["messages"]
    assert [m["role"] for m in messages] == ["user", "assistant"] and messages[-1]["content"].strip() == expected
    print("CLI first turn and persisted reply verified")
PY

printf '[real-cli-web] running tool chain and compaction through current model configuration\n'
python3 "$ROOT/scripts/test_real_cli_long_tasks.py" --binary "$BINARY" --config "$CONFIG_PATH" --case compaction

fi

printf '[real-cli-web] running token-session Web product path\n'
python3 - "$ROOT" "$BINARY" "$CONFIG_PATH" <<'PY'
import http.cookiejar
import json
import os
import shutil
import signal
import socket
import sqlite3
import subprocess
import sys
import tempfile
import time
import urllib.request
import uuid
from pathlib import Path

root, binary, config_path = sys.argv[1:]
expected = "NOLA_WEB_" + uuid.uuid4().hex
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
llm = runtime_config["llm"]
route = llm
if "connections" in llm:
    sys.path.insert(0, os.path.join(root, "scripts"))
    from e2e.current_model import declared_route
    route, _ = declared_route(runtime_config)
upstream_endpoint = route["endpoint"]
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
    route["endpoint"] = json.load(handle)["endpoint"]
with open(os.path.join(runtime_config_dir, "config.json"), "w", encoding="utf-8") as handle:
    json.dump(runtime_config, handle)
environment.update(HOME=runtime_home, NOLA_HOME=runtime_home, NOLA_MAC_APP_SUPPORT=os.path.join(runtime_dir, "support"))
environment["NOLA_DB_PATH"] = os.path.join(runtime_dir, "nola.sqlite")
if "connections" in llm:
    subprocess.run([binary, "model", "apply-config", os.path.join(runtime_config_dir, "config.json")],
                   cwd=runtime_dir, env=environment, check=True, stdout=subprocess.DEVNULL)
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
        os.path.join(root, "apps/web"),
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
            with opener.open(request, timeout=45) as response:
                return json.load(response)
        except urllib.error.HTTPError as error:
            payload = error.read().decode("utf-8", "replace")
            raise RuntimeError(
                f"HTTP {error.code} for {path}: {payload}"
            ) from error

    def encode_submission(request):
        conversation = request["conversationId"]
        request["expected_host_id"] = session["hostId"]
        config = json_request(f"/api/chat/conversations/{conversation}/configuration")
        request["expected_configuration_revision"] = config["configurationRevision"]
        history = json_request(f"/api/chat/conversations/{conversation}/messages")["messages"]
        request["expected_parent_message_id"] = history[-1]["messageId"] if history else None
        plan = json_request("/api/agent/tool-plan/prepare", {"expected_host_id": session["hostId"],
                            "client_id": request["client_id"], "session_id": conversation})
        request["tool_plan_ref"] = plan["result"]["reference"]
        identity = {key: request[key] for key in ("client_id", "submission_id", "user_message_id", "assistant_message_id")}
        identity.update(source_host_id=session["hostId"], conversation_id=conversation)
        prepared = json_request("/api/chat/agent/v2/submission", {"operation": "prepare", "identity": identity, "request": request})
        assert prepared["result"]["identity"] == identity
        request["request_digest"] = prepared["result"]["request_digest"]
        return json.dumps(request)

    def open_stream(request):
        try:
            return opener.open(request, timeout=180)
        except urllib.error.HTTPError as error:
            body = json.load(error)
            raise RuntimeError(f"Stream rejected (HTTP {error.code}): {body.get('code')}, {body.get('error')}") from error

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
        allowed_challenges=None,
    ):
        request = urllib.request.Request(
            f"{base_url}/api/chat/agent/v2/stream",
            data=encode_submission(
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
        with open_stream(request) as response:
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
                if allowed_challenges is not None and event.get("tool_name") not in allowed_challenges:
                    raise RuntimeError(f"Unexpected permission tool: {event.get('tool_name')}")
                if permission_decision not in {"allow", "deny"}:
                    raise RuntimeError(
                        "unexpected Web permission request: "
                        + json.dumps(event, ensure_ascii=False)
                    )
                decision_receipt = json_request(
                    "/api/agent/host-permission/decision",
                    {
                        "protocol_version": event.get("protocol_version", 0),
                        "nonce": event.get("nonce", ""),
                        "request_id": event.get("request_id") or event.get("id", ""),
                        "host_id": event.get("host_id", ""),
                        "client_id": event.get("client_id", ""),
                        "session_id": event.get("session_id", ""),
                        "tool_name": event.get("tool_name") or event.get("name", ""),
                        "resolution": {"type": "authorization", "decision": "allow_once" if permission_decision == "allow" else "deny"},
                    },
                )
                assert decision_receipt["accepted"] and decision_receipt["nonce"] == event["nonce"]
        return stream_events

    def canonical_trace(event):
        return event.get("trace", {}) if event.get("type") == "agent_trace" else {}

    def assistant_projection(events):
        notifications = [e for e in events if e.get("type") == "assistant_journal_committed_v1"]
        seals = [e for e in events if e.get("type") == "assistant_run_terminal_committed_v1"]
        if not notifications or len(seals) != 1:
            return "", 0
        seal = seals[0]
        rows = json_request(f"/api/chat/conversations/{seal['session_id']}/messages")["messages"]
        matching = [row for row in rows if row["role"] == "assistant"
                    and row["messageId"] == notifications[-1]["through"]["identity"]["assistant_message_id"]]
        assert len(matching) == 1
        return matching[0]["content"], len(notifications)

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
    goal_conversation = json_request("/api/chat/conversations", {"title": "Isolated Goal lifecycle"})
    goal_id = goal_conversation["conversationId"]
    goal_url = f"/api/chat/conversations/{goal_id}/goal"
    def control_goal(action, previous=None, **fields):
        body = {"action": action, "operationId": str(uuid.uuid4()), "expectedHostId": session["hostId"], **fields}
        if previous is not None:
            goal = previous["goalProjection"]
            body.update(expectedGoalId=goal["active"]["id"], expectedControlToken=goal["control_token"])
        receipt = json_request(goal_url, body)
        deadline = time.monotonic() + 45
        while time.monotonic() < deadline:
            operation = (receipt.get("goalProjection") or {}).get("applied_operations", {}).get(body["operationId"])
            if receipt.get("authoritative") and operation is not None:
                if operation.get("rejection") is not None:
                    raise RuntimeError(f"Goal {action} was rejected: {operation['rejection']}")
                return receipt
            time.sleep(0.2)
            receipt = json_request(goal_url)
        raise RuntimeError(f"Goal {action} did not reach a durable receipt")

    goal_start = control_goal("start", objective="Verify durable Web Goal lifecycle")
    goal_pause = control_goal("pause", goal_start)
    goal_schedule = control_goal("schedule", goal_pause, wakeAtEpochMs=int(time.time() * 1000) + 60_000)
    goal_resume = control_goal("resume", goal_schedule)
    goal_cancel = control_goal("cancel", goal_resume)
    goal_readback = json_request(goal_url)

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
        data=encode_submission(
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
    with open_stream(stream_request) as response:
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
    journal_indices = [i for i, e in enumerate(events) if e.get("type") == "assistant_journal_committed_v1"]
    seal_indices = [i for i, e in enumerate(events) if e.get("type") == "assistant_run_terminal_committed_v1"]
    committed_usage = json_request(f"/api/chat/conversations/{conversation_id}/context-usage")["usage"]
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

    recalled = stream_agent(branch_conversation_id,
        "Repeat the exact token in your immediately preceding reply. Use only conversation history, with no tools.",
        "real-web-branch-recall", agent_mode=False)
    branch_recall_exact = assistant_from(recalled).strip() == expected and not any(
        canonical_trace(event).get("kind") == "tool.started" for event in recalled)
    source_after_branch = json_request(f"/api/chat/conversations/{conversation_id}/messages")["messages"]
    source_unchanged = [(row["messageId"], row["content"]) for row in source_after_branch] == [
        (row["messageId"], row["content"]) for row in messages]
    if not branch_recall_exact or not source_unchanged:
        raise RuntimeError(f"Branch history check failed: recall={branch_recall_exact}, source_unchanged={source_unchanged}")

    report = {
        "loginAuthenticated": login.get("authenticated") is True,
        "sessionAuthenticated": session.get("sessionAuthenticated") is True,
        "eventCount": len(events),
        "assistantJournalUpdates": canonical_assistant_updates,
        "assistantResetCount": event_types.count("assistant_reset"),
        "thinkingDeltaCount": event_types.count("thinking_delta"),
        "streamCompleted": stream_done,
        "journalCommitCount": len(journal_indices),
        "runSealCount": len(seal_indices),
        "sealFollowsJournal": bool(journal_indices and seal_indices and seal_indices[0] > journal_indices[-1]),
        "committedContextUsedTokens": int(committed_usage.get("used_tokens", 0) or 0),
        "committedContextWindowTokens": int(committed_usage.get("context_window_tokens", 0) or 0),
        "assistantText": assistant_text,
        "traceRuns": len(traces.get("traces", [])),
        "storedRoles": [message.get("role") for message in messages],
        "branchCreated": bool(branch_conversation_id),
        "branchHasDistinctIdentity": bool(branch_conversation_id) and branch_conversation_id != conversation_id,
        "branchStoredRoles": [message.get("role") for message in branch_messages],
        "branchMessageCount": len(branch_messages),
        "branchHistoryRecalled": branch_recall_exact,
        "branchSourceUnchanged": source_unchanged,
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
    delete_prompt = (
        "Use the delete_file tool exactly once to delete the absolute file "
        f"{protected_path}. Do not use bash or another deletion tool. If permission "
        "is denied, do not retry. Tell me what happened."
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
        allowed_challenges={"delete_file"},
    )
    delete_text = assistant_from(delete_events)
    # The journal emits the denied call's own started/completed pair after the
    # permission is resolved. Verify distinct persisted calls and their decisions.
    with sqlite3.connect(Path(environment["NOLA_DB_PATH"]).as_uri() + "?mode=ro", uri=True) as db:
        tool_parts = [json.loads(raw) for raw, in db.execute(
            """SELECT p.content_json FROM message_parts p JOIN messages m ON m.id=p.message_id
               WHERE m.conversation_id=? AND p.kind='tool'""", (delete_conversation_id,))]
    deletions = [part for part in tool_parts if part["capability_id"] == "delete_file"]
    stopped_after_denial = (
        len(deletions) == 1 and deletions[0]["state"] == "failed"
        and (deletions[0].get("permission") or {}).get("decision") == "deny"
        and Path(deletions[0]["input"]["path"]).resolve() == Path(protected_path).resolve()
        and all(part["capability_id"] in {"tool_search", "delete_file"} for part in tool_parts)
    )
    if not stopped_after_denial:
        raise RuntimeError("Protected file operation lacked a single denied tool receipt")
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
            "noToolRetryAfterDenial": stopped_after_denial,
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

    memory_settings = json_request("/api/assistant/memory-default", {"enabled": True}, method="PUT")
    assert memory_settings["hostAllowed"] and memory_settings["enabled"]
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
        agent_mode=True, permission_decision="allow", allowed_challenges={"memory_save", "memory_search"},
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
        agent_mode=True, permission_decision="allow", allowed_challenges={"memory_search"},
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

    print("Completed Goal, data source, permission, memory and Skill flows", flush=True)
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
        marker = f"HISTORICAL_TURN_{index}_{uuid.uuid4().hex}"
        history_events = stream_agent(
            compact_conversation_id,
            f"Return exactly this ASCII token and nothing else: {marker}\n"
            + ("user-context " * 220),
            f"real-web-compaction-history-{index}",
            agent_mode=False,
        )
        if marker not in assistant_from(history_events):
            raise RuntimeError(f"compaction history turn {index} lost its marker")
    usage_path = f"/api/chat/conversations/{compact_conversation_id}/context-usage"
    usage_before = json_request(usage_path)["usage"]
    compact_result = json_request(
        f"/api/chat/conversations/{compact_conversation_id}/compact",
        {},
    )
    usage_after = json_request(usage_path)["usage"]
    continuation_expected = marker
    continuation_prompt = "Repeat the token in your most recent reply before compaction. Return only that token."
    continuation_request = urllib.request.Request(
        f"{base_url}/api/chat/agent/v2/stream",
        data=encode_submission(
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
            "compactContextUsage": {
                phase: {key: usage.get(key) for key in ("used_tokens", "context_window_tokens", "estimated", "measured_at_revision")}
                for phase, usage in (("before", usage_before), ("after", usage_after),
                                     ("nextModelTurn", json_request(usage_path)["usage"]))
            },
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
            "compactContinuationSeals": continuation_event_types.count(
                "assistant_run_terminal_committed_v1"
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
            report["assistantJournalUpdates"] >= 1,
            report["assistantResetCount"] == 0,
            report["streamCompleted"],
            report["journalCommitCount"] >= 1,
            report["runSealCount"] == 1,
            report["sealFollowsJournal"],
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
            bool(report["deleteAssistantText"]) and report["noToolRetryAfterDenial"],
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
            report["dataSkillStarted"] == 1,
            report["dataSkillRunCompleted"] == 1,
            report["dataSkillTraceOrdered"],
            report["dataSkillArtifactExists"],
            report["dataSkillArtifactHasRevenue"],
            report["dataSkillArtifactHasLimitations"],
            report["compactReplacedItemCount"] > 0,
            report["compactContinuationText"].strip()
            == continuation_expected,
            report["compactContinuationResetCount"] == 0,
            report["compactContinuationSeals"] == 1,
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
            "branch_history_recalled_without_changing_source": branch_recall_exact and source_unchanged,
            "permission_policy_verified": report["writePermissionRequests"] == 0 and stopped_after_denial and report["protectedFileText"] == "KEEP_WEB_PROTECTED",
            "cross_session_memory_verified": report["memoryRecallExactFormat"] and report["memorySaveCompleted"] == 1 and report["memorySearchCompleted"] >= 1,
            "data_source_read_and_disclosure_audited": report["dataSourceAnswerExact"] and report["dataSourceUsageAudited"],
            "skill_artifact_verified": report["dataSkillArtifactExists"] and report["dataSkillArtifactHasRevenue"] and report["dataSkillArtifactHasLimitations"],
            "compaction_continuation_verified": report["compactReplacedItemCount"] > 0 and report["compactContinuationText"].strip() == continuation_expected,
        }
        with open(audit_path, "w", encoding="utf-8") as handle:
            json.dump({"passed": all(audit_assertions.values()), "assertions": audit_assertions}, handle, ensure_ascii=False, indent=2)
        shutil.copyfile(provider_trace_path, trace_copy)
        with sqlite3.connect(environment["NOLA_DB_PATH"]) as source, sqlite3.connect(database_copy) as copy:
            source.backup(copy)
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
