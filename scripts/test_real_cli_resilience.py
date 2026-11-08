#!/usr/bin/env python3
"""Real-provider CLI resilience tests.

These tests intentionally exercise the installed provider configuration. They
cover failures and interactive concurrency that mock-server tests cannot prove.
"""

from __future__ import annotations

# Executable acceptance scenario. The system E2E runner supplies binary and
# project arguments; pytest must not reinterpret them as fixture names.
__test__ = False

import argparse
import contextlib
import hashlib
import json
import os
import pathlib
import signal
import sqlite3
import subprocess
import sys
import tempfile
import time
import uuid


from test_real_cli_terminal import Oracle, RealTerminal, exception_evidence, readonly
from e2e.model_execution_fixture import fixture, runtime_capabilities, strict_object
from e2e.provider_fault_relay import AuthenticationFaultRelay, ContextLimitFaultRelay
from canonical_run_audit import audit_runs


ORACLE_BINARY: pathlib.Path | None = None
CURRENT_STAGE = "startup"
DEBUG_SECRET = "nola_debug_secret_should_never_appear"


def sanitized_failure(error: BaseException) -> dict:
    value = {"case":"failure", "passed":False, "stage":CURRENT_STAGE,
        **exception_evidence(error)}
    value["error"] = value["message"]
    value["primary"] = exception_evidence(getattr(error, "primary_failure", None) or error)
    value["cleanup"] = [exception_evidence(failure) for failure in getattr(error, "cleanup_failures", [])]
    # The debug-only synthetic credential is not an upstream environment key.
    return json.loads(json.dumps(value).replace(DEBUG_SECRET, "[redacted]"))


@contextlib.contextmanager
def provider_fault(project: pathlib.Path, stage: str, relay_type, *relay_arguments):
    root = project / ".resilience-runtime"
    root.mkdir(parents=True, exist_ok=True)
    log = pathlib.Path(tempfile.mkdtemp(prefix=stage + "-", dir=root)) / "provider.ndjson"
    upstream = os.environ["NOLA_REAL_TEST_ENDPOINT"]
    declaration = fixture({"llm":{"endpoint":upstream,"model":os.environ["NOLA_REAL_TEST_MODEL"]}},
        dict(os.environ), require_tool=True)
    with relay_type(upstream, log, declaration["wireProtocol"],
            declaration["model"], *relay_arguments) as relay:
        # Both subprocesses see the same immutable route. This is scoped to a
        # sequential acceptance case, not an override of a persisted Run route.
        previous_log = os.environ.get("NOLA_REAL_TEST_PROVIDER_LOG")
        os.environ["NOLA_REAL_TEST_ENDPOINT"] = relay.endpoint
        os.environ["NOLA_REAL_TEST_PROVIDER_LOG"] = str(log)
        try:
            yield relay
        finally:
            os.environ["NOLA_REAL_TEST_ENDPOINT"] = upstream
            if previous_log is None:
                os.environ.pop("NOLA_REAL_TEST_PROVIDER_LOG", None)
            else:
                os.environ["NOLA_REAL_TEST_PROVIDER_LOG"] = previous_log


@contextlib.contextmanager
def authentication_fault(project: pathlib.Path, stage: str, *, local_secret: str | None = None):
    with provider_fault(project, stage, AuthenticationFaultRelay, local_secret) as relay:
        yield relay


def isolated_environment(project: pathlib.Path, permission_profile: str = "cautious") -> dict[str, str]:
    if permission_profile not in {"cautious", "allow_all"}:
        raise ValueError("permission_profile must be cautious or allow_all")
    root = project / ".resilience-runtime"
    paths = {"NOLA_HOME": root / "home", "NOLA_MAC_APP_SUPPORT": root / "app-support",
             "NOLA_EXTERNAL_EXECUTION_HOME": root / "execution-home"}
    for path in paths.values():
        path.mkdir(parents=True, exist_ok=True)
    environment = {key: value for key, value in os.environ.items()
                   if not key.startswith(("NOLA_", "nola_", "MODEL_", "OPENAI_", "ANTHROPIC_"))
                   and key not in {"DATABASE_URL", "DATABASE_PATH", "DB_PATH"}}
    environment.update({key: str(value) for key, value in paths.items()})
    environment.update(NOLA_DB_PATH=str(project / "nola-e2e.db"), NOLA_CLI_LANG="en",
                       NOLA_PERMISSION_PROFILE=permission_profile, NOLA_SKIP_ONBOARDING="1", TERM="xterm-256color")
    return environment


def clear_owned_credentials(database: pathlib.Path):
    # Only this exact DB's vault, never the DB/evidence or another DB's vault.
    vault = database.parent / ".nola-host-secrets"
    if vault.is_symlink():
        raise RuntimeError("owned credential directory became a symlink")
    if database.is_file() and vault.is_dir():
        name = hashlib.sha256(os.fsencode(database.resolve())).hexdigest() + ".sqlite"
        for suffix in ("", "-wal", "-shm", "-journal"):
            path = vault / (name + suffix)
            if path.is_symlink():
                raise RuntimeError("owned credential file became a symlink")
            if path.is_file():
                path.unlink()


@contextlib.contextmanager
def retained_project(directory: str):
    project = pathlib.Path(directory).resolve()
    project.mkdir(parents=True, exist_ok=True)
    print(json.dumps({"case": "retained-runtime", "path": str(project)}), flush=True)
    succeeded = False
    try:
        yield project
        succeeded = True
    finally:
        clear_owned_credentials(project / "nola-e2e.db")
    if succeeded:
        print(json.dumps({"case": "complete", "passed": True}))


@contextlib.contextmanager
def configured_environment(project: pathlib.Path, permission_profile: str = "cautious", *,
                           api_key_override: str | None = None, model_configuration: dict | None = None):
    environment = isolated_environment(project, permission_profile)
    root = pathlib.Path(tempfile.mkdtemp(prefix="profile-", dir=project / ".resilience-runtime"))
    folder = root / ".nola"
    folder.mkdir(mode=0o700)
    configuration = folder / "config.json"
    if model_configuration is None:
        arguments = provider_arguments()
        route = dict(zip(arguments[::2], arguments[1::2]))
        declaration = fixture({"llm": {"endpoint": route["--endpoint"], "model": route["--model"]}},
                              dict(os.environ), require_tool=True)
        model_configuration = {"llm": {"endpoint": route["--endpoint"], "model": route["--model"],
            "api_key": os.environ["NOLA_REAL_TEST_API_KEY"], "provider": {"chat_completions": "openai-compatible",
                "responses": "openai-responses", "anthropic": "anthropic"}[declaration["wireProtocol"]],
            "capabilities": runtime_capabilities(declaration)}}
    llm = model_configuration.get("llm", {})
    if set(model_configuration) != {"llm"} or set(llm) != {"endpoint", "model", "api_key", "provider", "capabilities"}:
        raise ValueError("only a minimal explicit model fixture may be written")
    llm = dict(llm)
    if api_key_override is not None:
        llm["api_key"] = api_key_override
    try:
        descriptor = os.open(configuration, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(descriptor, "w", encoding="utf-8") as output:
            json.dump({"llm": llm}, output)
        environment["NOLA_HOME"] = str(root)
        yield environment
    finally:
        if configuration.is_symlink():
            raise RuntimeError("owned configuration became a symlink")
        if configuration.is_file():
            configuration.unlink()


def run_outcomes(database: pathlib.Path, session: str, oracle: Oracle | None = None, *,
                 live: bool = False) -> list[str | None]:
    """Cold typed seal validation after exit; live Service reads are explicit.

    Never retry a failed live read as a cold read or implicitly start a Service.
    """
    if not database.is_file():
        return []
    owned = oracle is None
    if owned:
        if ORACLE_BINARY is None:
            raise AssertionError("current terminal acceptance oracle is required")
        oracle = Oracle(ORACLE_BINARY)
    try:
        audit = oracle.query(operation="audit", database=str(database), live=live)
        return [run["kind"] for run in audit["runs"] if run["conversationId"] == session]
    finally:
        if owned:
            oracle.close()


def provider_arguments() -> list[str]:
    values = {
        "endpoint": os.environ.get("NOLA_REAL_TEST_ENDPOINT", "").strip(),
        "model": os.environ.get("NOLA_REAL_TEST_MODEL", "").strip(),
        "api_key": os.environ.get("NOLA_REAL_TEST_API_KEY", "").strip(),
    }
    missing = [key for key, value in values.items() if not value]
    if missing:
        raise AssertionError(f"missing real-provider test settings: {', '.join(missing)}")
    return [
        "--endpoint",
        values["endpoint"],
        "--model",
        values["model"],
    ]


def provider_request_count() -> int:
    raw = os.environ.get("NOLA_REAL_TEST_PROVIDER_LOG", "").strip()
    if not raw:
        raise AssertionError("NOLA_REAL_TEST_PROVIDER_LOG is required")
    path = pathlib.Path(raw)
    if not path.exists():
        return 0
    count = 0
    for line in path.read_text(encoding="utf-8").splitlines():
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if event.get("requestId") or event.get("request_id"):
            count += 1
    return count


def provider_statuses_since(start: int) -> list[int]:
    raw = os.environ.get("NOLA_REAL_TEST_PROVIDER_LOG", "").strip()
    if not raw:
        raise AssertionError("NOLA_REAL_TEST_PROVIDER_LOG is required")
    path = pathlib.Path(raw)
    if not path.exists():
        return []
    statuses: list[int] = []
    events = []
    for line in path.read_text(encoding="utf-8").splitlines():
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if event.get("requestId") or event.get("request_id"):
            events.append(event)
    for event in events[start:]:
        status = event.get("status")
        if isinstance(status, int):
            statuses.append(status)
    return statuses


def latest_provider_failure() -> str:
    raw = os.environ.get("NOLA_REAL_TEST_PROVIDER_LOG", "").strip()
    if not raw:
        return "provider log unavailable"
    path = pathlib.Path(raw)
    if not path.exists():
        return "provider log missing"
    for line in reversed(path.read_text(encoding="utf-8").splitlines()):
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        status = event.get("status")
        error = event.get("error")
        if status or error:
            return f"status={status!r} error={error!r}"[:1000]
    return "provider log contains no response event"


def diagnostic_tool_parts(database: pathlib.Path, session: str) -> list[dict[str, object]]:
    """Read latest bounded Part diagnostics, not canonical tool input/output.

    These run_events previews support lifecycle/capability assertions below.
    They do not prove preservation of the original canonical tool JSON body.
    """
    if not database.exists():
        return []
    latest_diagnostic_by_part: dict[str, dict[str, object]] = {}
    with readonly(database) as connection:
        rows = connection.execute(
            """
            SELECT e.payload_json
            FROM run_events e
            JOIN runs r ON r.id = e.run_id
            WHERE r.conversation_id = ? AND e.event_kind = 'part_updated'
            ORDER BY e.sequence
            """,
            (session,),
        )
        for (raw,) in rows:
            try:
                diagnostic_part = json.loads(raw).get("part", {})
            except json.JSONDecodeError:
                continue
            if diagnostic_part.get("kind") != "tool":
                continue
            part_id = str(
                diagnostic_part.get("id")
                or diagnostic_part.get("call_id")
                or len(latest_diagnostic_by_part)
            )
            latest_diagnostic_by_part[part_id] = diagnostic_part
    return list(latest_diagnostic_by_part.values())


def command_receipt_snapshot(database: pathlib.Path) -> dict:
    with readonly(database) as connection:
        connection.execute("BEGIN")
        records = {(client, request):{"type":kind,"status":status,"command":json.loads(command),
            "result":json.loads(result) if result is not None else None}
            for client,request,kind,status,command,result in connection.execute(
                "SELECT client_id,request_id,command_type,status,command_json,result_json FROM command_receipts")}
        groups = [{"type":kind,"status":status,"count":count} for kind,status,count in connection.execute(
            "SELECT command_type,status,count(*) FROM command_receipts GROUP BY command_type,status ORDER BY command_type,status")]
    return {"records":records,"groups":groups}


def audit_authentication_failure(database: pathlib.Path, session: str) -> dict:
    """Corroborate an Oracle-validated Failed seal, never human error wording."""
    with readonly(database) as connection:
        connection.execute("BEGIN")
        assert audit_runs(connection,session) == {"runCount":1,"outcomes":["failed"]}
        run, = connection.execute("SELECT id FROM runs WHERE conversation_id=? AND kind='conversation'",(session,)).fetchone()
        result = json.loads(connection.execute("SELECT result_json FROM run_results WHERE run_id=?",(run,)).fetchone()[0])["value"]
        assert isinstance(result.get("error_ref"),str) and result["error_ref"]
        hosts = []
        for effect,request,raw in connection.execute(
                "SELECT effect_id,request_id,payload_json FROM host_event_inbox WHERE run_id=?",(run,)):
            envelope = json.loads(raw)
            if envelope.get("payload",{}).get("type") == "failed":
                assert envelope.get("effect_id") == effect and envelope.get("request_id") == request
                assert envelope.get("session_id") == session and envelope.get("turn_id") == run
                hosts.append(envelope)
        assert len(hosts) == 1, "authentication failure lacks a unique same-owner Host failure"
        host = hosts[0]
        failure = host["payload"].get("error",{})
        assert failure.get("code") == "authentication_failed" and type(failure.get("http_status")) is int
        assert failure["http_status"] == 401 and failure.get("retryable") is False
        assert isinstance(failure.get("message"),str) and failure["message"]
        assert isinstance(host.get("host_id"),str) and host["host_id"]
        invocations = list(connection.execute("SELECT config_json FROM run_invocation_payloads WHERE run_id=?",(run,)))
        assert len(invocations) == 1, "failure lacks its unique admitted Host configuration"
        invocation = json.loads(invocations[0][0])
        assert invocation.get("session_id") == session and invocation.get("host_catalog",{}).get("host_id") == host["host_id"], "failure belongs to another admitted Host"
        attempts = list(connection.execute("SELECT id,effect_id,record_json FROM run_attempts WHERE run_id=?",(run,)))
        assert len(attempts) == 1, "401 must fail one exact model Attempt without retry"
        identity,effect,raw = attempts[0]
        attempt = json.loads(raw)
        assert attempt.get("id") == identity == host.get("attempt_id")
        assert attempt.get("run_id") == run and attempt.get("effect_id") == effect == host["effect_id"]
        assert attempt.get("invocation_id") == effect and attempt.get("state") == "failed"
        assert attempt.get("failure") == failure, "Host and model Attempt authentication failure disagree"
        bindings = list(connection.execute("SELECT request_id FROM run_attempt_deliveries WHERE attempt_id=?",(identity,)))
        assert bindings == [(host["request_id"],)], "failure is not bound to the exact model request"
        delivery = host.get("delivery",{})
        deliveries = list(connection.execute("SELECT effect_id,request_id,state,record_json FROM effect_delivery_attempts "
            "WHERE id=? AND run_id=?",(delivery.get("attempt_id"),run)))
        assert len(deliveries) == 1, "Host failure lacks its durable physical delivery"
        effect,request,state,raw = deliveries[0]
        physical = json.loads(raw)
        assert effect == host["effect_id"] and request == host["request_id"] and state == "settled"
        assert physical.get("id") == delivery["attempt_id"] and physical.get("run_id") == run
        assert physical.get("effect_id") == effect and physical.get("request_id") == request and physical.get("state") == state
        assert all(isinstance(delivery.get(key),str) and delivery[key] and delivery[key] == physical.get(key)
            for key in ("worker_generation","lease_owner","lease_token")), "Host failure delivery fence mismatch"
        events = [(kind,json.loads(raw)) for kind,raw in connection.execute(
            "SELECT event_kind,payload_json FROM run_events WHERE run_id=? ORDER BY sequence",(run,))]
        facts = [event for kind,event in events if kind == "runtime_decision_fact_observed"
            and event.get("fact",{}).get("fact_id") == result.get("error_ref")]
        terminals = [event for kind,event in events if kind == "turn_failed"]
        assert len(facts) == len(terminals) == 1, "Failed seal lacks its exact runtime error fact/terminal"
        assert facts[0].get("type") == "runtime_decision_fact_observed" and terminals[0].get("type") == "turn_failed"
        for event in (facts[0],terminals[0]):
            assert event.get("turn_id") == run and event.get("core_envelope",{}).get("turn_id") == run
            assert event.get("core_envelope",{}).get("session_id") == session
        assert facts[0]["fact"].get("kind") == {"kind":"error","code":"runtime_failure","detail":failure["message"]}
        assert terminals[0].get("session_id") == session and terminals[0].get("error") == failure["message"]
        assert result.get("observed_effect_refs") == [] and result.get("unknown_effect_refs") == []
        return {"runID":run,"attemptID":identity,"effectID":host["effect_id"],"requestID":host["request_id"],
            "failureCode":"authentication_failed","httpStatus":401,"runKind":"failed"}


def audit_wait_receipts(before: dict, after: dict, session: str, active_run: str | None) -> dict:
    original, current = before["records"], after["records"]
    assert original.keys() <= current.keys(), "wait removed durable command receipts"
    changed = {identity:record for identity,record in current.items() if original.get(identity) != record}
    counts = {}
    for record in changed.values():
        kind, status, command, result = record["type"], record["status"], record["command"], record["result"]
        counts[(kind,status)] = counts.get((kind,status),0) + 1
        assert kind != "claim_next_agent_command", "empty wait created an AgentCommand Claim receipt"
        if kind.startswith("claim_"):
            assert result not in (None,[],{}), "empty wait created a null/empty Claim receipt"
        assert status == "committed", "wait produced an unfinished or rejected write receipt"
        if kind == "heartbeat_effect_delivery":
            renewal = command.get("input",{})
            assert active_run and renewal.get("run_id") == active_run, "unrelated lease renewal"
            assert isinstance(result,dict) and result.get("attempt_id") == renewal.get("attempt_id")
            assert all(isinstance(renewal.get(key),str) and renewal[key] and result.get(key) == renewal[key]
                for key in ("worker_generation","lease_owner","lease_token")), "invalid renewal result identity"
        elif kind == "record_agent_trace":
            assert command.get("event",{}).get("conversation_id") == session, "unrelated trace during wait"
        elif kind not in {"open_assistant_journal_snapshot","release_assistant_journal_snapshot"}:
            raise AssertionError(f"unexpected command during empty wait: {kind}")
    return {"beforeGroups":before["groups"],"afterGroups":after["groups"],
        "newReceiptIDs":[{"client_id":client,"request_id":request,"type":current[(client,request)]["type"],
            "status":current[(client,request)]["status"]} for client,request in sorted(current.keys()-original.keys())],
        "changedExistingCount":sum(identity in original for identity in changed),
        "deltaGroups":[{"type":kind,"status":status,"count":count} for (kind,status),count in sorted(counts.items())],
        "emptyAgentClaimReceipts":0,"leaseRenewalCount":sum(count for (kind,_status),count in counts.items()
            if kind == "heartbeat_effect_delivery")}


def audit_queued_command(before: dict, after: dict, session: str, text: str) -> dict:
    records = [record for identity,record in after["records"].items() if identity not in before["records"]]
    enqueued = []
    for record in records:
        if record["type"] != "enqueue_agent_command" or record["command"].get("input",{}).get("session_id") != session:
            continue
        source = record["command"]["input"]
        submission = json.loads(source["payload_json"])
        if any(item.get("type") == "message" and item.get("role") == "user" and item.get("content") == text
                for item in submission.get("items",[])):
            assert record["status"] == "committed" and record["result"] is True
            assert submission.get("type") == "user_input" and submission.get("delivery") == "next_turn"
            assert source["command_id"] == submission.get("submission_id") and bool(source["command_id"])
            enqueued.append(source)
    assert len(enqueued) == 1, "queued user input lacks its unique original enqueue receipt"
    source = enqueued[0]
    claims = [record for record in records if record["type"] == "claim_next_agent_command"
        and isinstance(record["result"],dict) and record["result"].get("command_id") == source["command_id"]]
    assert len(claims) == 1, "queued command lacks a unique nonempty Claim"
    claim, lease = claims[0], claims[0]["result"]
    owner = lease.get("lease_owner")
    assert claim["status"] == "committed" and lease.get("session_id") == session
    assert claim["command"].get("session_id") == session and claim["command"].get("lease_owner") == owner
    assert isinstance(owner,str) and owner and type(lease.get("delivery_attempt")) is int and lease["delivery_attempt"] == 1
    # Enqueue admits AgentSubmission; Application stores exactly one typed
    # AgentBoundaryInput::Submission wrapper. Compare its entire current wire
    # value, preserving JSON scalar types (Python bool == int is insufficient).
    claimed = strict_object(lease["payload_json"])
    expected = {"type":"submission","submission":strict_object(source["payload_json"])}
    assert json.dumps(claimed,sort_keys=True,ensure_ascii=False,allow_nan=False) == json.dumps(
        expected,sort_keys=True,ensure_ascii=False,allow_nan=False), "Claim changed the admitted Submission envelope"
    assert type(source.get("created_at_ms")) is int and type(lease.get("created_at_ms")) is int
    assert lease["created_at_ms"] == source["created_at_ms"]
    acknowledgements = [record for record in records if record["type"] == "acknowledge_agent_command"
        and record["command"].get("command_id") == source["command_id"]]
    assert len(acknowledgements) == 1, "queued command lacks its unique ACK, not a permission response"
    ack = acknowledgements[0]
    assert ack["status"] == "committed" and ack["result"] is True
    assert ack["command"].get("session_id") == session and ack["command"].get("lease_owner") == owner
    return {"commandID":source["command_id"],"leaseOwner":owner,"deliveryAttempt":lease["delivery_attempt"],
        "nonemptyClaim":True,"originalCommandAcknowledged":True}


def observe_empty_wait(cli, binary: pathlib.Path, project: pathlib.Path,
                       target: pathlib.Path, *, pending: bool) -> dict:
    def projection():
        result = configured_run(binary, project, ["session","show",cli.session,"--json"])
        assert result.returncode == 0, "formal wait projection read failed"
        value = json.loads(result.stdout)
        assert value.get("id") == cli.session, "formal permission projection belongs to another conversation"
        permissions = value.get("permission_projection")
        assert isinstance(permissions,dict) and isinstance(permissions.get("pending"),list)
        return permissions
    def living_owner(stage):
        members = cli.terminal._owned_group(stage)
        assert not cli.terminal.reaped and not cli.terminal.pty_closed
        assert any(member["pid"] == cli.pid and not member["state"].startswith("Z") for member in members), "wait owner exited"
    original = target.read_bytes()
    permissions = projection()
    requests = permissions["pending"]
    assert len(requests) == (1 if pending else 0), "wait did not begin in the required formal permission state"
    active_run = requests[0]["turn_id"] if pending else None
    if pending:
        request = requests[0]
        assert request.get("request_id") and request.get("call_id") and request.get("capability_id") == "write_file"
        subject = json.loads(request["resource"])
        assert any(resource.get("kind") == "file" and resource.get("access") == "write"
            and resource.get("components") == list(target.resolve().parts[1:]) for resource in subject.get("resources",[]))
    living_owner("q01-before")
    before = command_receipt_snapshot(cli.database)
    started = time.monotonic()
    while time.monotonic()-started < 30:
        cli.terminal.pump()
        assert not cli.terminal.pty_closed, "PTY exited during the no-input window"
        assert target.read_bytes() == original, "pending/idle wait changed original bytes"
    elapsed = time.monotonic()-started
    living_owner("q01-after")
    assert projection() == permissions, "formal permission request/revision changed during no-input wait"
    after = command_receipt_snapshot(cli.database)
    assert target.read_bytes() == original
    return {"phase":"permission-pending" if pending else "composer-idle","durationSeconds":round(elapsed,3),
        "ownerAlive":True,"originalBytesUnchanged":True,"sameFormalPermissionState":True,
        **audit_wait_receipts(before,after,cli.session,active_run)}


def audit_denied_file_invocation(database: pathlib.Path, session: str,
                               target: pathlib.Path, original: bytes) -> dict:
    """Corroborate an Oracle-validated Blocked Run with exact durable facts.

    Bounded Part previews cannot establish permission decisions. Host resource
    facts bind the call/target; the formal settlement and Host response bind
    the request and decision. No prunable pending/outbox payload is required.
    """
    def require(condition: bool, message: str):
        if not condition:
            raise AssertionError(message)
    with readonly(database) as connection:
        connection.execute("BEGIN")
        require(audit_runs(connection, session) == {"runCount":1,"outcomes":["blocked"]},
                "denied invocation requires one canonical Blocked Run")
        run, assistant = connection.execute("SELECT id,assistant_message_id FROM runs "
            "WHERE conversation_id=? AND kind='conversation'", (session,)).fetchone()
        result = json.loads(connection.execute("SELECT result_json FROM run_results WHERE run_id=?", (run,)).fetchone()[0])
        require(result["value"].get("reason", {}).get("code") == "permission.denied",
                "Blocked outcome is not a permission denial")
        events = [(sequence, kind, json.loads(raw)) for sequence, kind, raw in connection.execute(
            "SELECT sequence,event_kind,payload_json FROM run_events WHERE run_id=? ORDER BY sequence", (run,))]
        settlements = [(sequence, event) for sequence, kind, event in events if kind == "permission_settled"]
        require(len(settlements) == 1, "missing or duplicate formal permission settlement")
        settled_sequence, settled = settlements[0]
        receipt = settled.get("receipt", {})
        call, request = receipt.get("call_id"), receipt.get("request_id")
        require(settled.get("type") == "permission_settled"
                and settled.get("core_envelope", {}).get("session_id") == session
                and settled.get("core_envelope", {}).get("turn_id") == run
                and receipt.get("turn_id") == run and receipt.get("capability_id") == "write_file"
                and isinstance(call, str) and bool(call) and isinstance(request, str) and bool(request)
                and receipt.get("outcome") == {"type":"authorization","decision":"deny"},
                "formal permission denial has another owner/call/capability or decision")
        latest = {}
        for sequence, kind, event in events:
            if kind != "part_updated" or event.get("part", {}).get("kind") != "tool":
                continue
            part = event["part"]
            require(part.get("turn_id") == run and part.get("message_id") == assistant,
                    "tool Part has another denial owner")
            latest[part["id"]] = (sequence, part)
        require(len(latest) == 1, "denial has missing or substitute tool calls")
        final_sequence, part = next(iter(latest.values()))
        require(part.get("call_id") == call and part.get("capability_id") == "write_file"
                and part.get("state") == "failed" and final_sequence > settled_sequence,
                "denied exact tool call must be Failed after settlement, not Cancelled")
        host = []
        for effect, rpc, raw in connection.execute(
                "SELECT effect_id,request_id,payload_json FROM host_event_inbox WHERE run_id=?", (run,)):
            envelope = json.loads(raw)
            require(envelope.get("effect_id") == effect and envelope.get("request_id") == rpc
                    and envelope.get("session_id") == session and envelope.get("turn_id") == run
                    and isinstance(envelope.get("delivery"), dict),
                    "Host response differs from its stored owner/effect/request identity or lacks delivery")
            host.append(envelope)
        responses = [envelope for envelope in host if envelope.get("payload", {}).get("type") == "permission_resolved"]
        require(len(responses) == 1, "missing or duplicate Host permission resolution")
        response = responses[0]
        require(response.get("session_id") == session and response.get("turn_id") == run
                and response.get("payload") == {"type":"permission_resolved","request_id":request,
                    "resolution":{"type":"authorization","decision":"deny"}},
                "Host permission resolution has another owner/request or decision")
        resources = [envelope for envelope in host if envelope.get("payload", {}).get("type") == "permission_resources_resolved"]
        require(len(resources) == 1, "missing or duplicate Host permission resource facts")
        resolved = resources[0]
        facts = resolved.get("payload", {}).get("facts", {})
        subject = json.loads(receipt.get("resource", "null"))
        require(isinstance(subject, dict) and resolved.get("session_id") == session
                and resolved.get("turn_id") == run and resolved["payload"].get("call_id") == call
                and isinstance(response.get("host_id"), str) and bool(response["host_id"])
                and resolved.get("host_id") == response["host_id"] == facts.get("execution_target")
                and subject.get("execution_target") == response["host_id"]
                and subject.get("capability") == "write_file" and subject.get("operation") == "filesystem",
                "Host resource facts have another owner/call/target/capability")
        bindings = facts.get("bindings", [])
        require(len(bindings) == 1 and subject.get("resources") == [bindings[0].get("resource")],
                "settled permission resource differs from Host facts")
        binding, path = bindings[0], str(target.resolve(strict=True))
        resource, demand = binding.get("resource", {}), binding.get("demand", {})
        require(resource.get("kind") == "file" and resource.get("access") == "write"
                and bool(resource.get("filesystem_id")) and resource.get("components") == list(target.resolve().parts[1:])
                and demand.get("locator_kind") == "file_path" and demand.get("access") == "write"
                and demand.get("locator") == path and binding.get("observation") == "overwrite",
                "formal denial is not the exact existing fixture overwrite")
        require(not any(kind == "execution_evidence_observed" for _, kind, _ in events)
                and not any(envelope.get("payload", {}).get("type") == "tool_completed" for envelope in host),
                "denied invocation has execution evidence or a completed tool")
        require(connection.execute("SELECT count(*) FROM effect_outbox WHERE run_id=? AND effect_kind='execute_tool'", (run,)).fetchone()[0] == 0,
                "denied invocation dispatched an ExecuteTool effect")
        # Outbox rows/payloads can be pruned. Every durable delivery must have
        # exact Host evidence proving it was an allowed non-tool protocol.
        attempts = {identity:(effect,rpc,state,json.loads(raw)) for identity,effect,rpc,state,raw in connection.execute(
            "SELECT id,effect_id,request_id,state,record_json FROM effect_delivery_attempts WHERE run_id=?", (run,))}
        for identity,(effect,rpc,state,record) in attempts.items():
            require(record.get("id") == identity and record.get("run_id") == run
                    and record.get("effect_id") == effect and record.get("request_id") == rpc
                    and state == record.get("state") == "settled",
                    "denial delivery identity/state is unknown or unsettled")
            sources = [envelope for envelope in host if envelope.get("effect_id") == effect
                and envelope.get("request_id") == rpc and envelope.get("session_id") == session
                and envelope.get("turn_id") == run and envelope.get("host_id") == response["host_id"]
                and envelope.get("delivery", {}).get("attempt_id") == identity]
            require(bool(sources), "durable attempt lacks exact Host source after outbox pruning")
            for envelope in sources:
                require(all(envelope["delivery"].get(key) == record.get(key) and isinstance(record.get(key), str)
                    and bool(record[key]) for key in ("worker_generation","lease_owner","lease_token")),
                    "denial Host delivery fence differs from its durable attempt")
            types = {envelope.get("payload", {}).get("type") for envelope in sources}
            if effect == response["effect_id"]:
                allowed = {"execution_started","permission_resolved"}
                complete = "permission_resolved"
            elif effect == resolved["effect_id"]:
                allowed = {"execution_started","permission_resources_resolved"}
                complete = "permission_resources_resolved"
            else:
                allowed = {"execution_started","model_attempt_started","tool_progress","model_delta",
                    "model_committed_delta","model_continuation","model_completed"}
                complete = "model_completed"
            require(complete in types and types <= allowed,
                    "durable attempt is not a proven model/resource/permission operation")
        require(all(envelope["delivery"].get("attempt_id") in attempts
            and attempts[envelope["delivery"]["attempt_id"]][:2] == (envelope["effect_id"],envelope["request_id"])
            and envelope.get("host_id") == response["host_id"] for envelope in host),
                "Host response lacks its exact durable delivery attempt/Host")
        require(target.read_bytes() == original, "denied invocation changed its original bytes")
        return {"run_id":run,"call_id":call,"request_id":request,"decision":"deny",
            "tool_state":"failed","run_kind":"blocked","reason":"permission.denied",
            "physical_executions":0,"permission_request_count":1}


def canonical_part_text(connection: sqlite3.Connection, part_id: str,
                        inline_text: str | None, inline_json: str | None) -> str:
    """Read a canonical text Part, including immutable backing when present."""
    fields = {"content_text": inline_text, "content_json": inline_json}
    for field, reference_json in connection.execute(
        "SELECT field, reference_json FROM message_part_body_references WHERE part_id = ?",
        (part_id,),
    ):
        reference = json.loads(reference_json)
        chunks = connection.execute(
            "SELECT text FROM canonical_text_chunks WHERE content_id = ? ORDER BY offset_bytes",
            (reference["content_id"],),
        )
        fields[field] = "".join(row[0] for row in chunks)
    content = fields["content_text"] or ""
    if not content and fields["content_json"]:
        typed_part = json.loads(fields["content_json"])
        if typed_part.get("kind") == "text":
            content = typed_part.get("text", "")
    return content


def canonical_text_parts(database: pathlib.Path, session: str) -> list[tuple[str, str]]:
    if not database.exists():
        return []
    with readonly(database) as connection:
        result = []
        for part_id, role, inline_text, inline_json in connection.execute(
                """
                SELECT p.id, m.role, p.content_text, p.content_json
                FROM messages m
                JOIN message_parts p ON p.message_id = m.id
                WHERE m.conversation_id = ? AND p.kind = 'text'
                ORDER BY m.ordinal, p.ordinal
                """,
                (session,),
            ):
            content = canonical_part_text(connection, part_id, inline_text, inline_json)
            if content:
                result.append((str(role), content))
        return result


def canonical_run_status(database: pathlib.Path, session: str) -> str | None:
    if not database.exists():
        return None
    with readonly(database) as connection:
        row = connection.execute(
            "SELECT status FROM runs WHERE conversation_id = ? ORDER BY created_at_ms DESC LIMIT 1",
            (session,),
        ).fetchone()
    return None if row is None else str(row[0])


class InteractiveCLI:
    def __init__(
        self,
        binary: pathlib.Path,
        project: pathlib.Path,
        session: str,
        *,
        permission_profile: str = "cautious",
    ) -> None:
        self.database = project / "nola-e2e.db"
        self.session = session
        isolated_environment(project, permission_profile)  # reject invalid modes before spawning
        if ORACLE_BINARY is None:
            raise AssertionError("current terminal acceptance oracle is required")
        arguments = [str(binary), *provider_arguments(), "--project", str(project),
                     "--debug", "--session", session, "--lang", "en"]
        root = project / ".resilience-runtime" / f"terminal-{session}-{uuid.uuid4().hex}"
        root.mkdir(parents=True)
        self.configuration = configured_environment(project, permission_profile)
        environment = self.configuration.__enter__()
        self.oracle = None
        try:
            self.oracle = Oracle(ORACLE_BINARY)
            self.terminal = RealTerminal(binary, project, environment, self.oracle, root, arguments=arguments)
        except BaseException:
            if self.oracle is not None:
                self.oracle.close()
            self.configuration.__exit__(None, None, None)
            raise
        self.fd, self.pid = self.terminal.master, self.terminal.pid
        self.output = self.terminal.output

    def send(self, value: str) -> None:
        os.write(self.fd, value.encode("utf-8") + b"\r")

    def choose_and_wait(self, digit: str, needle: str, timeout: float) -> str:
        """Confirm a numeric choice, observing only output after this action."""
        assert digit in "123456789" and len(digit) == 1
        offset = len(self.output)
        os.write(self.fd, digit.encode("ascii"))
        return self.wait_for(needle, timeout, start=offset)

    def wait_for_answer(self, count: int, timeout: float = 150) -> str:
        """Only a validated seal/result proves completion, never a composer."""
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            self.drain_until_quiet(quiet_for=0.05, timeout=0.2)
            if not self.database.exists():
                continue
            outcomes = run_outcomes(self.database, self.session, self.oracle, live=True)
            if len(outcomes) < count or outcomes[-1] is None:
                continue
            assert outcomes[-1] in {"answered", "action_completed"}, outcomes
            with sqlite3.connect(f"file:{self.database}?mode=ro", uri=True) as connection:
                answers = connection.execute(
                    "SELECT id,status FROM messages WHERE conversation_id=? AND role='assistant' ORDER BY ordinal",
                    (self.session,),
                ).fetchall()
                if len(answers) < count:
                    continue
                identity, _status = answers[-1]
                return "\n".join(canonical_part_text(connection, part_id, inline, typed)
                    for part_id, inline, typed in connection.execute(
                        "SELECT id,content_text,content_json FROM message_parts WHERE message_id=? AND kind='text' ORDER BY ordinal",
                        (identity,),
                    ))
        raise AssertionError("canonical assistant answer did not complete")

    def wait_for_terminal(self, count: int, timeout: float = 60) -> list[str]:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            self.drain_until_quiet(quiet_for=0.05, timeout=0.2)
            outcomes = run_outcomes(self.database, self.session, self.oracle, live=True)
            if len(outcomes) == count and all(outcome is not None for outcome in outcomes):
                return outcomes
        raise AssertionError("formal terminal Run receipt did not settle")

    def send_escape(self) -> None:
        os.write(self.fd, b"\x1b")

    def frames_since(self, start: int) -> list[dict]:
        return [frame for frame in self.terminal.frames if frame["rawOffset"] > start]

    def wait_for(self, needle: str, timeout: float, start: int = 0) -> str:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            for frame in self.frames_since(start):
                if needle in frame["screen"]:
                    return frame["screen"]
            self.terminal.pump()
            if self.terminal.pty_closed:
                break
        raise AssertionError(
            f"timed out waiting for committed frame {needle!r}; "
            f"terminal tail:\n{self.terminal.screen()[-200:]}"
        )

    def wait_for_clean(self, needle: str, timeout: float, start: int = 0) -> str:
        return self.wait_for(needle, timeout, start)

    def wait_for_any(self, needles: tuple[str, ...], timeout: float, start: int = 0) -> tuple[str, str]:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            for frame in self.frames_since(start):
                for needle in needles:
                    if needle in frame["screen"]:
                        return needle, frame["screen"]
            self.terminal.pump()
            if self.terminal.pty_closed:
                break
        raise AssertionError(
            f"timed out waiting for committed frame {needles!r}; "
            f"terminal tail:\n{self.terminal.screen()[-200:]}"
        )

    def drain_until_quiet(self, quiet_for: float = 0.25, timeout: float = 2.0) -> str:
        deadline = time.monotonic() + timeout
        quiet_deadline = time.monotonic() + quiet_for
        while time.monotonic() < deadline and time.monotonic() < quiet_deadline:
            before = len(self.output)
            self.terminal.pump()
            if self.terminal.pty_closed:
                break
            if len(self.output) != before:
                quiet_deadline = time.monotonic() + quiet_for
        return self.terminal.screen()

    def close(self) -> None:
        self.cleanup_failures = []
        for stage, close in (("terminal", self.terminal.close), ("oracle", self.oracle.close),
                             ("configuration", lambda: self.configuration.__exit__(None, None, None))):
            try:
                close()
            except Exception as failure:
                detail = {"stage": stage, "failure": exception_evidence(failure)}
                if stage == "terminal":
                    detail["processFailures"] = getattr(self.terminal, "cleanup_failures", [])
                self.cleanup_failures.append(detail)
        if self.cleanup_failures:
            raise RuntimeError("Interactive CLI cleanup failed; original scenario error is retained separately")

    def __enter__(self) -> "InteractiveCLI":
        try:
            self.wait_for("Type a message", 10)
        except BaseException:
            self.__exit__(*sys.exc_info())
            raise
        return self

    def __exit__(self, _kind: object, _value: object, _traceback: object) -> None:
        primary = exception_evidence(_value, _traceback) if _value is not None else None
        cleanup = []
        try:
            if _kind is not None:
                # Keep failed real-provider evidence too; otherwise provider
                # errors and incomplete terminal frames vanish on teardown.
                self.database.with_name(f"{self.session}-failed-terminal.raw").write_bytes(self.output)
        except Exception as failure:
            cleanup.append({"stage": "failure_capture", "failure": exception_evidence(failure)})
        finally:
            try:
                self.close()
            except Exception as failure:
                cleanup.extend(getattr(self, "cleanup_failures", []) or
                    [{"stage": "close", "failure": exception_evidence(failure)}])
        if primary is not None or cleanup:
            report = self.database.with_name(f"{self.session}-failure.json")
            evidence = {"primary": primary, "cleanup": cleanup}
            try:
                descriptor = os.open(report, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
                with os.fdopen(descriptor, "w", encoding="utf-8") as stream:
                    json.dump(evidence, stream, ensure_ascii=False, indent=2)
            except Exception as failure:
                cleanup.append({"stage": "failure_report", "failure": exception_evidence(failure)})
                print(json.dumps(evidence, ensure_ascii=False), file=sys.stderr)
            if cleanup:
                original = f"Primary {primary['type']}: {primary['message']}; " if primary else ""
                raise RuntimeError(original + f"cleanup failed; evidence={report}") from _value


def configured_run(binary: pathlib.Path, project: pathlib.Path, arguments: list[str], *,
                   timeout: float = 60, input_text: str | None = None,
                   api_key_override: str | None = None, environment_overrides: dict | None = None):
    with configured_environment(project, api_key_override=api_key_override) as environment:
        environment.update(environment_overrides or {})
        return subprocess.run([str(binary), *provider_arguments(), *arguments], cwd=project,
            env=environment, input=input_text, text=True, stdout=subprocess.PIPE,
            stderr=subprocess.PIPE, timeout=timeout, check=False)


def run_process(
    binary: pathlib.Path,
    project: pathlib.Path,
    args: list[str],
    *,
    stage: str = "noninteractive",
    api_key_override: str | None = None,
) -> subprocess.CompletedProcess[str]:
    global CURRENT_STAGE
    if stage != "noninteractive":
        CURRENT_STAGE = stage
    started = time.monotonic()
    try:
        result = configured_run(binary, project, args, api_key_override=api_key_override)
    except subprocess.TimeoutExpired as error:
        stdout = error.stdout.decode(errors="replace") if isinstance(error.stdout, bytes) else (error.stdout or "")
        stderr = error.stderr.decode(errors="replace") if isinstance(error.stderr, bytes) else (error.stderr or "")
        raise AssertionError(
            f"real CLI stage timed out: stage={stage!r} "
            f"elapsed={time.monotonic() - started:.1f}s "
            f"provider={latest_provider_failure()} "
            f"stdout_tail={stdout[-800:]!r} stderr_tail={stderr[-800:]!r}"
        ) from error
    print(
        json.dumps(
            {
                "case": "scenario-timing",
                "stage": stage,
                "durationSeconds": round(time.monotonic() - started, 3),
                "exit": result.returncode,
            }
        )
    )
    return result


def test_provider_ready(binary: pathlib.Path, project: pathlib.Path) -> None:
    marker = "NOLA_REAL_PROVIDER_READY_7319"
    session = f"readiness-{uuid.uuid4().hex}"
    result = run_process(
        binary,
        project,
        [
            "--json",
            "--no-tools",
            "--session",
            session,
            f"Reply with exactly {marker} and nothing else.",
        ],
        stage="provider-ready",
    )
    combined = result.stdout + result.stderr
    assert result.returncode == 0, (
        "real provider readiness failed before CLI interaction tests; "
        f"exit={result.returncode}; provider={latest_provider_failure()}"
    )
    # JSON transport publishes journal commit cursors, not duplicate body text.
    # Confirm the durable assistant body, never a prompt/trace echo of the marker.
    assistant = "".join(text for role, text in canonical_text_parts(
        project / "nola-e2e.db", session
    ) if role == "assistant")
    assert assistant.strip() == marker and run_outcomes(project / "nola-e2e.db", session) == ["answered"], (
        "real provider returned success without the readiness marker; "
        f"provider={latest_provider_failure()}; cli_tail={combined[-500:]}"
    )
    print(json.dumps({"case": "provider-ready", "passed": True}))


def test_auth_failure_and_recovery(binary: pathlib.Path, project: pathlib.Path) -> None:
    failed_session = f"auth-failed-{uuid.uuid4().hex}"
    recovered_session = failed_session
    with authentication_fault(project, "auth-recovery") as relay:
        failed = run_process(binary, project, ["--json", "--no-tools", "--session", failed_session,
            "Reply with AUTH_SHOULD_NOT_SUCCEED only."], stage="auth-injected-401")
        relay.assert_injected()
        assert failed.returncode != 0, "injected HTTP 401 must produce a non-zero exit status"
        assert "AUTH_SHOULD_NOT_SUCCEED" not in failed.stdout
        assert run_outcomes(project / "nola-e2e.db", failed_session) == ["failed"]
        authentication = audit_authentication_failure(project / "nola-e2e.db",failed_session)
        assert not any("AUTH_SHOULD_NOT_SUCCEED" in text for role,text in canonical_text_parts(
            project / "nola-e2e.db",failed_session) if role == "assistant"), "failed auth produced a canonical fake answer"
        recovered = run_process(binary, project, ["--json", "--no-tools", "--session", recovered_session,
            "Reply with exactly AUTH_RECOVERED_7319 and nothing else."], stage="auth-real-recovery")
        assert recovered.returncode == 0, recovered.stderr
        relay.assert_recovered()
    assert any(text == "AUTH_RECOVERED_7319" for role, text in canonical_text_parts(
        project / "nola-e2e.db", recovered_session) if role == "assistant")
    assert run_outcomes(project / "nola-e2e.db", recovered_session) == ["failed", "answered"]
    print(
        json.dumps(
            {
                "case": "auth-failure-recovery",
                "failedExit": failed.returncode,
                "recoveredExit": recovered.returncode,
                "authenticationFailure": authentication,
            }
        )
    )


def test_queued_follow_up(binary: pathlib.Path, project: pathlib.Path) -> None:
    session = f"queue-{uuid.uuid4().hex}"
    # This scenario measures queued delivery, not authorization.
    # The sleep-only fixture explicitly selects the product's all-allowed mode.
    with InteractiveCLI(binary, project, session, permission_profile="allow_all") as cli:
        cli.send(
            "You must use the bash tool to run exactly: sleep 4. "
            "After it finishes, reply FIRST_DONE_7319."
        )
        cli.wait_for("thinking", 20)
        offset = len(cli.output)
        before_queue = command_receipt_snapshot(cli.database)
        follow_up = "Reply with exactly QUEUED_FOLLOWUP_7319 and nothing else."
        cli.send(follow_up)
        # The terminal renderer owns indentation and viewport placement.
        cli.wait_for_clean("FIRST_DONE_7319", 90, start=offset)
        expected_text = [
            (
                "user",
                "You must use the bash tool to run exactly: sleep 4. "
                "After it finishes, reply FIRST_DONE_7319.",
            ),
            ("assistant", "FIRST_DONE_7319"),
            ("user", "Reply with exactly QUEUED_FOLLOWUP_7319 and nothing else."),
            ("assistant", "QUEUED_FOLLOWUP_7319"),
        ]
        database = project / "nola-e2e.db"
        deadline = time.monotonic() + 90
        while time.monotonic() < deadline:
            # Keep consuming the PTY so viewport redraw traffic cannot
            # backpressure the child while the canonical successor settles.
            cli.drain_until_quiet(quiet_for=0.02, timeout=0.1)
            if canonical_text_parts(database, session) == expected_text:
                break
        else:
            raise AssertionError(
                "queued successor did not reach canonical assistant completion: "
                f"status={canonical_run_status(database, session)!r}, "
                f"parts={canonical_text_parts(database, session)!r}"
            )
        rendered = cli.drain_until_quiet(quiet_for=0.25, timeout=3)
        for marker in ["FIRST_DONE_7319", "QUEUED_FOLLOWUP_7319", "sleep 4"]:
            assert any(marker in frame["screen"] for frame in cli.terminal.frames), marker
        outcomes = run_outcomes(database, session, cli.oracle, live=True)
        assert len(outcomes) == 2 and outcomes[0] in {"answered", "action_completed"} and outcomes[1] == "answered", outcomes
        queue_receipt = audit_queued_command(before_queue,command_receipt_snapshot(database),session,follow_up)
    print(
        json.dumps(
            {
                "case": "queued-follow-up",
                "firstCompleted": True,
                "followUpCompleted": True,
                "queueReceipt": queue_receipt,
            }
        )
    )


def test_permission_custom_response(binary: pathlib.Path, project: pathlib.Path) -> None:
    session = f"permission-{uuid.uuid4().hex}"
    target = project / "custom-probe.txt"
    target.write_text("ORIGINAL_UNCHANGED", encoding="utf-8")
    database = project / "nola-e2e.db"
    with InteractiveCLI(binary, project, session, permission_profile="cautious") as cli:
        cli.send(
            "Overwrite the existing file named custom-probe.txt with exactly ORIGINAL_CONTENT "
            "using write_file."
        )
        cli.wait_for("Return to composer", 30)
        cli.choose_and_wait("3", "Type a message", 10)
        assert cli.wait_for_terminal(1) == ["cancelled"]
        cli.drain_until_quiet()
        cli.send(
            "Do not overwrite the file. Reply with exactly "
            "CUSTOM_PERMISSION_7319 and nothing else."
        )
        deadline = time.monotonic() + 45
        transcript: list[tuple[str, str]] = []
        statuses: list[str] = []
        while time.monotonic() < deadline:
            cli.drain_until_quiet(quiet_for=0.05, timeout=0.1)
            transcript = canonical_text_parts(database, session)
            statuses = run_outcomes(database, session, cli.oracle, live=True)
            if (
                ("assistant", "CUSTOM_PERMISSION_7319") in transcript
                and statuses == ["cancelled", "answered"]
            ):
                break
            time.sleep(0.1)
        assert target.read_text() == "ORIGINAL_UNCHANGED", "custom response must prevent the overwrite"
        assert transcript.count(("user", "Do not overwrite the file. Reply with exactly CUSTOM_PERMISSION_7319 and nothing else.")) == 1, transcript
        assert transcript.count(("assistant", "CUSTOM_PERMISSION_7319")) == 1, transcript
        assert statuses == ["cancelled", "answered"], statuses
    print(
        json.dumps(
            {
                "case": "permission-custom-response",
                "fileCreated": False,
                "modelFollowedReplacement": True,
            }
        )
    )


def test_allow_once_prompts_again(binary: pathlib.Path, project: pathlib.Path) -> None:
    session = f"allow-once-{uuid.uuid4().hex}"
    target = project / "allow-once-probe.txt"
    target.write_text("BEFORE_ALLOW_ONCE", encoding="utf-8")
    with InteractiveCLI(binary, project, session, permission_profile="cautious") as cli:
        cli.send(
            f"Overwrite the existing file using write_file with path exactly {target} and content exactly FIRST_ALLOW_ONCE."
        )
        cli.wait_for("Return to composer", 40)
        pending_wait = observe_empty_wait(cli,binary,project,target,pending=True)
        print(json.dumps({"case":"q01-empty-wait",**pending_wait}),flush=True)
        first_choice = len(cli.output)
        assert target.read_text() == "BEFORE_ALLOW_ONCE", "write ran before permission approval"
        # Numeric shortcuts confirm immediately; do not hide regressions by
        # sending Enter after the digit as send() would do.
        os.write(cli.fd, b"1")
        cli.wait_for_answer(1, 150)
        cli.wait_for("Type a message", 40, start=first_choice)
        assert target.read_text() == "FIRST_ALLOW_ONCE"
        idle_wait = observe_empty_wait(cli,binary,project,target,pending=False)
        print(json.dumps({"case":"q01-empty-wait",**idle_wait}),flush=True)

        second_turn = len(cli.output)
        cli.send(
            f"Use write_file again with path exactly {target} and content exactly "
            "SECOND_REVISION."
        )
        rendered = cli.wait_for("Return to composer", 40, start=second_turn)
        assert "write_file" in rendered or str(target) in rendered, rendered[-5000:]
        cli.choose_and_wait("3", "Type a message", 20)
        cli.drain_until_quiet(quiet_for=0.25, timeout=2)
        # A repainted composer is not evidence that the pending operation was
        # cancelled. Verify the durable terminal state before teardown.
        deadline = time.monotonic() + 10
        statuses = []
        while time.monotonic() < deadline:
            statuses = run_outcomes(cli.database, session, cli.oracle, live=True)
            if statuses[0:1] in [["answered"], ["action_completed"]] and statuses[1:] == ["cancelled"]:
                break
            cli.drain_until_quiet(quiet_for=0.05, timeout=0.2)
        assert statuses[0:1] in [["answered"], ["action_completed"]] and statuses[1:] == ["cancelled"], statuses
        assert target.read_text() == "FIRST_ALLOW_ONCE"
        (project / "allow-once-terminal.raw").write_bytes(cli.output)
    print(
        json.dumps(
            {
                "case": "allow-once-prompts-again",
                "secondPromptShown": True,
                "secondSideEffectPrevented": True,
                "idleWait": idle_wait,
                "pendingWait": pending_wait,
            }
        )
    )


def test_deny_has_no_side_effect_or_tool_substitution(
    binary: pathlib.Path, project: pathlib.Path
) -> None:
    session = f"deny-{uuid.uuid4().hex}"
    target = project / "deny-probe.txt"
    target.write_text("BEFORE_DENY", encoding="utf-8")
    database = project / "nola-e2e.db"
    with InteractiveCLI(binary, project, session, permission_profile="cautious") as cli:
        cli.send(
            f"Overwrite the existing file using write_file with path exactly {target} and content exactly DENIED_CONTENT."
        )
        cli.wait_for("Return to composer", 40)
        decision_offset = len(cli.output)
        cli.send("n")
        cli.wait_for("Type a message", 40, start=decision_offset)
        cli.drain_until_quiet(quiet_for=1.0, timeout=4.0)
        assert target.read_text() == "BEFORE_DENY", "denied invocation changed its target"
        assert cli.wait_for_terminal(1) == ["blocked"], "Deny must seal Blocked, not Cancelled"
    diagnostic_tools = diagnostic_tool_parts(database, session)
    capabilities = [str(part.get("capability_id") or "") for part in diagnostic_tools]
    assert capabilities.count("write_file") == 1, capabilities
    assert "bash" not in capabilities, capabilities
    denial = audit_denied_file_invocation(database, session, target, b"BEFORE_DENY")
    print(
        json.dumps(
            {
                "case": "deny-no-substitution",
                "sideEffectPrevented": True,
                "bashSubstitution": False,
                "permissionRequestCount": denial["permission_request_count"],
            }
        )
    )


def test_external_workspace_access_and_followup(
    binary: pathlib.Path, project: pathlib.Path
) -> None:
    session = f"external-workspace-{uuid.uuid4().hex}"
    # The target is deliberately NOT inside the starting workspace.
    with tempfile.TemporaryDirectory(prefix="nola-external-target-") as directory:
        target = pathlib.Path(directory).resolve()
        fixture = target / "evidence.txt"
        fixture.write_text("NOLA_EXTERNAL_EVIDENCE_5921\n", encoding="utf-8")
        with InteractiveCLI(binary, project, session, permission_profile="cautious") as cli:
            cli.send(f"请读取 {fixture}，用中文说明文件内容。只读，不要修改任何文件。")
            cli.wait_for("Return to composer", 120)
            diagnostic_parts = diagnostic_tool_parts(project / "nola-e2e.db", session)
            request_diagnostics = [part for part in diagnostic_parts if part.get("capability_id") == "workspace_access"]
            assert request_diagnostics, diagnostic_parts
            assert fixture.read_text() == "NOLA_EXTERNAL_EVIDENCE_5921\n"
            cli.send("y")
            answer = cli.wait_for_answer(1, 150)
            assert "NOLA_EXTERNAL_EVIDENCE_5921" in answer, answer
            assert any("\u4e00" <= character <= "\u9fff" for character in answer), answer
            cli.send("继续，确认刚才读取的是目标目录，而不是会话临时目录。不要重复申请已经授予的权限。")
            followup = cli.wait_for_answer(2, 120)
            assert any("\u4e00" <= character <= "\u9fff" for character in followup), followup
        diagnostic_parts = diagnostic_tool_parts(project / "nola-e2e.db", session)
        request_diagnostics = [part for part in diagnostic_parts if part.get("capability_id") == "workspace_access"]
        assert len(request_diagnostics) == 1, request_diagnostics
        assert fixture.read_text() == "NOLA_EXTERNAL_EVIDENCE_5921\n"
    print(json.dumps({"case": "external-workspace-access-followup", "passed": True}))


def test_read_only_inspection_needs_no_permission(
    binary: pathlib.Path, project: pathlib.Path
) -> None:
    session = f"read-only-{uuid.uuid4().hex}"
    fixture = project / "read-only-fixture.txt"
    fixture.write_text("NOLA_READ_ONLY_FIXTURE_7319\n", encoding="utf-8")
    database = project / "nola-e2e.db"
    with InteractiveCLI(binary, project, session, permission_profile="cautious") as cli:
        offset = len(cli.output)
        cli.send(
            f"Read {fixture} using the read_file tool. Then reply with exactly "
            "READ_ONLY_INSPECTION_7319. Do not modify anything."
        )
        assert cli.wait_for_answer(1, 50).strip() == "READ_ONLY_INSPECTION_7319"
        assert not any("Return to composer" in frame["screen"] for frame in cli.frames_since(offset))
    assert fixture.read_text(encoding="utf-8") == "NOLA_READ_ONLY_FIXTURE_7319\n"
    diagnostic_tools = diagnostic_tool_parts(database, session)
    capabilities = [str(part.get("capability_id") or "") for part in diagnostic_tools]
    assert capabilities and set(capabilities) <= {"read_file"}, capabilities
    assert not any(part.get("state") == "waiting_for_permission" for part in diagnostic_tools), diagnostic_tools
    print(
        json.dumps(
            {
                "case": "read-only-no-permission",
                "capabilities": capabilities,
                "permissionRequested": False,
                "fixtureUnchanged": True,
            }
        )
    )


def test_complete_diff_discovery_and_natural_followup(
    binary: pathlib.Path, project: pathlib.Path
) -> None:
    """Use only synthetic files; never audit or commit the developer's worktree."""
    repository = project / "evidence-worktree"
    repository.mkdir()

    def git(*args: str) -> bytes:
        return subprocess.run(
            ["git", *args], cwd=repository, check=True, capture_output=True, timeout=20
        ).stdout

    git("init")
    (repository / "large.txt").write_text("baseline\n", encoding="utf-8")
    (repository / "staged.txt").write_text("baseline\n", encoding="utf-8")
    git("add", ".")
    git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "-m", "fixture")
    (repository / "large.txt").write_text("large changed line\n" * 4000 + "UNSTAGED_TAIL_7319\n", encoding="utf-8")
    (repository / "staged.txt").write_text("STAGED_TAIL_8420\n", encoding="utf-8")
    git("add", "staged.txt")
    (repository / "new.txt").write_text("UNTRACKED_TAIL_9531\n", encoding="utf-8")
    (repository / "artifact.bin").write_bytes(b"\0\xffBINARY_FIXTURE")
    before = git("status", "--porcelain=v1", "-z")
    before_head = git("rev-parse", "HEAD")
    before_index = git("diff", "--cached")
    before_contents = {path.name: path.read_bytes() for path in repository.iterdir() if path.is_file()}
    session = f"evidence-contract-{uuid.uuid4().hex}"
    database = project / "nola-e2e.db"
    # This scenario tests evidence and follow-ups, not permission decisions.
    # Explicit approval applies only inside this synthetic fixture; the separate
    # Cautious permission cases separately exercise real ask/allow/deny.
    with InteractiveCLI(binary, project, session, permission_profile="allow_all") as cli:
        cli.send(
            f"只读审查 {repository} 的当前改动。先用 tool_search 查询 "
            "select:git_diff,read_fiel（第二个名称可能拼错，按目录返回结果恢复），"
            "然后实际调用 git_diff。报告每个文本文件修改后的最后一行，"
            "并区分 staged、unstaged、untracked 以及二进制文件。不要修改文件或执行提交。"
            "回答最后单独一行写 EVIDENCE_AUDIT_DONE。"
        )
        answer = cli.wait_for_answer(1)
        for marker in ["UNSTAGED_TAIL_7319", "STAGED_TAIL_8420", "UNTRACKED_TAIL_9531", "artifact.bin"]:
            assert marker in answer, (marker, answer[-6000:])
        cli.send("Now explain the same findings briefly in English. End with EVIDENCE_FOLLOWUP_OK. Still do not change any files.")
        assert "EVIDENCE_FOLLOWUP_OK" in cli.wait_for_answer(2)
    assert git("status", "--porcelain=v1", "-z") == before
    assert git("rev-parse", "HEAD") == before_head
    assert git("diff", "--cached") == before_index
    assert {path.name: path.read_bytes() for path in repository.iterdir() if path.is_file()} == before_contents
    diagnostic_tools = diagnostic_tool_parts(database, session)
    capabilities = [str(part.get("capability_id") or "") for part in diagnostic_tools]
    assert "tool_search" in capabilities and "git_diff" in capabilities, capabilities
    assert not any(part.get("state") == "waiting_for_permission" for part in diagnostic_tools), diagnostic_tools
    print(json.dumps({"case": "complete-diff-discovery-followup", "passed": True, "capabilities": capabilities}))


def test_multilingual_ambiguous_words_do_not_force_tools(
    binary: pathlib.Path, project: pathlib.Path
) -> None:
    session = f"ambiguous-routing-{uuid.uuid4().hex}"
    database = project / "nola-e2e.db"
    with InteractiveCLI(binary, project, session, permission_profile="cautious") as cli:
        cli.send(
            "只回复 AMBIGUOUS_ZH_7319。这里的 search 和 current 只是要解释的英文单词，"
            "不是要求联网、搜索或读取文件。"
        )
        assert cli.wait_for_answer(1, timeout=40).strip() == "AMBIGUOUS_ZH_7319"
        cli.send(
            "Natural follow-up: reply with exactly AMBIGUOUS_EN_7319. "
            "The quoted phrase 'current search state' is text, not a request to use a tool."
        )
        assert cli.wait_for_answer(2, timeout=40).strip() == "AMBIGUOUS_EN_7319"
    diagnostic_tools = diagnostic_tool_parts(database, session)
    assert diagnostic_tools == [], diagnostic_tools
    print(
        json.dumps(
            {
                "case": "multilingual-ambiguous-no-forced-tools",
                "toolCount": 0,
                "naturalFollowUpCompleted": True,
            }
        )
    )


def test_goal_return_to_composer_stops_successors(
    binary: pathlib.Path, project: pathlib.Path
) -> None:
    session = f"goal-permission-{uuid.uuid4().hex}"
    target = project / "must-not-overwrite.txt"
    target.write_text("GOAL_ORIGINAL_UNCHANGED", encoding="utf-8")
    database = project / "nola-e2e.db"
    with InteractiveCLI(binary, project, session, permission_profile="cautious") as cli:
        cli.send(
            "/goal Overwrite existing must-not-overwrite.txt with content FORBIDDEN. "
            "Use write_file and do not claim completion before the actual overwrite."
        )
        cli.wait_for("Return to composer", 60)
        cli.choose_and_wait("3", "Type a message", 20)
        assert cli.wait_for_terminal(1) == ["cancelled"]
        cli.drain_until_quiet(quiet_for=1.0, timeout=5.0)
        before_provider = provider_request_count()
        with readonly(database) as connection:
            before_runs = connection.execute(
                "SELECT COUNT(*) FROM runs WHERE conversation_id = ?", (session,)
            ).fetchone()[0]
        time.sleep(4.0)
        cli.drain_until_quiet(quiet_for=0.5, timeout=2.0)
        after_provider = provider_request_count()
        with readonly(database) as connection:
            after_runs = connection.execute(
                "SELECT COUNT(*) FROM runs WHERE conversation_id = ?", (session,)
            ).fetchone()[0]
            cancellation_reasons = [
                json.loads(row[0]).get("reason")
                for row in connection.execute(
                    """
                    SELECT e.payload_json
                    FROM run_events e
                    JOIN runs r ON r.id = e.run_id
                    WHERE r.conversation_id = ? AND e.event_kind = 'turn_cancelled'
                    """,
                    (session,),
                )
            ]
        assert target.read_text() == "GOAL_ORIGINAL_UNCHANGED", "returning to composer must prevent the overwrite"
        assert after_runs == before_runs, (before_runs, after_runs)
        assert after_provider == before_provider, (before_provider, after_provider)
        assert "returned_to_composer" in cancellation_reasons, cancellation_reasons
    print(
        json.dumps(
            {
                "case": "goal-return-to-composer-stops-successors",
                "sideEffectPrevented": True,
                "successorRunCreated": False,
                "providerRequestAfterReturn": False,
            }
        )
    )


def test_session_permission_survives_restart(binary: pathlib.Path, project: pathlib.Path) -> None:
    session = f"permission-restart-{uuid.uuid4().hex}"
    first_target = project / "session-overwrite.txt"
    first_target.write_text("BEFORE_SESSION_APPROVAL", encoding="utf-8")
    second_target = first_target
    with InteractiveCLI(binary, project, session, permission_profile="cautious") as first:
        first.send(
            f"Overwrite the existing file using write_file with path exactly {first_target} and content exactly "
            "FIRST_SESSION_WRITE. Do not choose another path."
        )
        first.wait_for("Return to composer", 30)
        selection_offset = len(first.output)
        os.write(first.fd, b"2")
        first.wait_for_answer(1)
        first.wait_for("Type a message", 40, start=selection_offset)
        assert first_target.read_text() == "FIRST_SESSION_WRITE"

    with InteractiveCLI(binary, project, session, permission_profile="cautious") as second:
        offset = len(second.output)
        second.send(
            f"Overwrite the same existing file using write_file with path exactly {second_target} and content exactly "
            "SECOND_SESSION_WRITE. Do not choose another path."
        )
        outcome, rendered = second.wait_for_any(
            ("Return to composer", "Type a message"),
            40,
            start=offset,
        )
        assert outcome == "Type a message", rendered[-5000:]
        second.wait_for_answer(2)
        assert not any("Return to composer" in frame["screen"] for frame in second.frames_since(offset))
        assert second_target.read_text() == "SECOND_SESSION_WRITE"
    print(
        json.dumps(
            {
                "case": "session-permission-restart",
                "promptedAfterRestart": False,
                "secondWriteCompleted": True,
            }
        )
    )


def test_failed_turn_session_recovery(binary: pathlib.Path, project: pathlib.Path) -> None:
    session = f"failed-turn-{uuid.uuid4().hex}"
    failed_prompt = "FAILED_TURN_MARKER_7319"
    recovery_prompt = "Reply with exactly SAME_SESSION_RECOVERED_7319 and nothing else."
    with authentication_fault(project, "failed-turn") as relay:
        failed = run_process(binary, project, ["--json", "--session", session, "--no-tools", failed_prompt],
            stage="failed-turn-injected-401")
        relay.assert_injected()
        assert failed.returncode != 0, "injected HTTP 401 did not fail the initial Turn"
        assert run_outcomes(project / "nola-e2e.db", session) == ["failed"]
        authentication = audit_authentication_failure(project / "nola-e2e.db",session)
        recovered = run_process(binary, project,
            ["--json", "--session", session, "--no-tools", recovery_prompt], stage="failed-turn-real-recovery")
        assert recovered.returncode == 0, recovered.stderr
        relay.assert_recovered()
    assert any(text == "SAME_SESSION_RECOVERED_7319" for role, text in canonical_text_parts(
        project / "nola-e2e.db", session) if role == "assistant")
    assert run_outcomes(project / "nola-e2e.db", session)[-1] == "answered"

    shown = run_process(
        binary,
        project,
        ["session", "show", session, "--json"],
    )
    assert shown.returncode == 0, shown.stderr
    messages = json.loads(shown.stdout)["messages"]
    role_content = [
        (message.get("role"), message.get("content"))
        for message in messages
        if message.get("role") in {"user", "assistant"}
    ]
    assert role_content == [
        ("user", failed_prompt),
        ("assistant", ""),
        ("user", recovery_prompt),
        ("assistant", "SAME_SESSION_RECOVERED_7319"),
    ], role_content
    failed_assistant = messages[1]
    assert failed_assistant.get("parts"), failed_assistant
    assert failed_assistant["parts"][0].get("state") == "failed", failed_assistant
    print(
        json.dumps(
            {
                "case": "failed-turn-session-recovery",
                "failedAssistantRecorded": True,
                "failedAssistantExcludedFromTextContext": True,
                "recovered": True,
                "authenticationFailure": authentication,
            }
        )
    )


def test_escape_records_cancelled_turn(binary: pathlib.Path, project: pathlib.Path) -> None:
    session = f"cancel-{uuid.uuid4().hex}"
    with InteractiveCLI(binary, project, session, permission_profile="allow_all") as cli:
        cli.send(
            "Use the bash tool to run exactly: sleep 20. "
            "After it finishes, reply SHOULD_NOT_COMPLETE_7319."
        )
        cli.wait_for("thinking", 20)
        offset = len(cli.output)
        cli.send_escape()
        rendered = cli.wait_for("Type a message", 20, start=offset)
        assert all("SHOULD_NOT_COMPLETE_7319" not in text for role, text in canonical_text_parts(
            project / "nola-e2e.db", session) if role == "assistant")
        assert cli.wait_for_terminal(1) == ["cancelled"]
        live_receipt = [run for run in cli.oracle.query(operation="audit", database=str(cli.database), live=True)["runs"]
                        if run["conversationId"] == session]

    # Exact owner/seal/result was validated while the real CLI Service was live.
    with contextlib.closing(Oracle(ORACLE_BINARY)) as oracle:
        cold_receipt = [run for run in oracle.query(operation="audit", database=str(project / "nola-e2e.db"), live=False)["runs"]
                        if run["conversationId"] == session]
    assert cold_receipt == live_receipt
    print(
        json.dumps(
            {
                "case": "escape-cancelled-turn",
                "cancelledTerminal": True,
                "failedTerminal": False,
            }
        )
    )


def audit_debug_log_segment(log: str, *, terminal: str, forbidden: tuple[str, ...]) -> dict:
    """Diagnostic identities are layered; this is not a durable Run seal audit."""
    assert all(value and value not in log for value in forbidden), "debug log disclosed protected text"
    records = []
    for line in log.splitlines():
        prefix, separator, raw = line.partition(" payload=")
        assert separator, "debug record has no structured payload"
        words = prefix.split()
        assert words and words[0] in {"nola_llm_debug", "nola_agent_debug"}, "unknown debug record"
        fields = {}
        for word in words[1:]:
            key, equals, value = word.partition("=")
            assert equals and key not in fields, "malformed debug prefix"
            fields[key] = value
        payload = strict_object(raw)
        phase = fields.get("phase")
        assert phase, "debug record has no phase"
        run = payload.get("run_id", fields.get("run_id"))
        assert isinstance(run, str) and run, "debug record has no invocation identity"
        if "run_id" in payload and "run_id" in fields:
            assert payload["run_id"] == fields["run_id"], "debug invocation identity mismatch"
        execution = payload.get("execution_id", fields.get("execution_id"))
        if isinstance(execution, str) and execution.isdecimal():
            execution = int(execution)
        if phase in {"prompt_composition", "agent_terminal", "request_start", "provider_response",
                     "request_http_error", "model_provider_failure"}:
            assert type(execution) is int and execution > 0, "debug execution identity missing"
        if "execution_id" in fields and "execution_id" in payload:
            assert fields["execution_id"] == str(execution), "debug execution identity mismatch"
        records.append((phase, run, execution, payload))
    assert records and len({record[1] for record in records}) == 1, "mixed debug invocations"

    def phase(name):
        return [(index, record) for index, record in enumerate(records) if record[0] == name]

    compositions, terminals = phase("prompt_composition"), phase("agent_terminal")
    assert len(compositions) == len(terminals) == 1, "missing or duplicate controller boundary"
    composition_index, composition = compositions[0]
    terminal_index, settled = terminals[0]
    assert composition[2] == settled[2], "controller execution identity mismatch"
    assert settled[3].get("type") == "agent_terminal" and settled[3].get("reason") == terminal, "wrong debug terminal"
    starts = phase("request_start")
    responses = phase("provider_response" if terminal == "answered" else "request_http_error")
    assert starts and len(starts) == len(responses), "missing or duplicate provider boundary"
    previous = composition_index
    for (start_index, start), (response_index, response) in zip(starts, responses):
        assert previous < start_index < response_index < terminal_index, "debug phases out of order"
        assert start[2] == response[2], "provider execution identity mismatch"
        if terminal == "answered":
            assert response[3].get("type") == "provider_response", "wrong provider response type"
        previous = response_index
    if terminal == "answered":
        assert not phase("request_http_error") and not phase("model_provider_failure"), "recovery provider failed"
    else:
        assert terminal == "failed" and not phase("provider_response"), "wrong failure evidence"
        failures = phase("model_provider_failure")
        assert len(failures) == 1, "missing authentication failure"
        index, failure = failures[0]
        # Dispatch returns the provider error to the controller; this event is
        # emitted in that controller scope, not inside the HTTP execution.
        assert responses[-1][0] < index < terminal_index and failure[2] == composition[2], "foreign failure evidence"
        assert (failure[3].get("code"), failure[3].get("status"), failure[3].get("retryable")) == (
            "authentication_failed", 401, False), "wrong authentication failure"
    return {"terminal": terminal, "modelRequests": len(starts), "sameInvocation": True,
        "controllerIdentityMatched": True, "providerIdentitiesMatched": True}


def audit_debug_human_answer(stdout: str, marker: str) -> None:
    # PIPE output is not a TTY and has no color/spinner. Recognize only the
    # current renderer's initial clear-line and four-space assistant indent;
    # do not strip arbitrary ANSI or find an echoed prompt with `in`.
    # This exact Core-control presentation is formatting, never evidence of
    # authorization or successful business execution. Accept at most one.
    receipt = "    • Tool · requirements_update\n\n"
    if stdout.startswith(receipt):
        stdout = stdout[len(receipt):]
        assert stdout.startswith("\x1b[2K"), "unexpected control receipt wrapping"
        stdout = stdout[len("\x1b[2K"):]
    else:
        # emit_markdown_text writes clear_current_output_line = CR + CSI 2 K;
        # configured_run(text=True) performs universal-newline CR -> LF decoding.
        for prefix in ("\r\x1b[2K", "\n\x1b[2K"):
            if stdout.startswith(prefix):
                stdout = stdout[len(prefix):]
                break
    text = stdout.strip("\n").removeprefix("    ")
    assert not any(ord(character) < 32 for character in text), "unexpected human output control"
    assert text == marker, "no-save human answer differs from the exact requested text"


def capture_debug_human_answer(project: pathlib.Path, stdout: str, secrets: tuple[str, ...]) -> None:
    """Retain this synthetic response only, with credentials scrubbed on failure."""
    assert secrets and all(secrets), "missing debug capture redaction identity"
    disclosed = any(secret in stdout for secret in secrets)
    sanitized = stdout
    for secret in secrets:
        sanitized = sanitized.replace(secret, "[redacted]")
    path = project / "real-debug-answer.stdout.json"
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, "w", encoding="utf-8") as stream:
        json.dump({"stdout":sanitized, "credentialDisclosure":disclosed,
            "transport":"human_pipe_universal_newlines", "durableSealProven":False}, stream)
    assert not disclosed, "debug stdout disclosed a credential; retained capture is redacted"


def test_debug_log_real_request(binary: pathlib.Path, project: pathlib.Path) -> None:
    log_path = project / "real-debug.jsonl"
    secret = DEBUG_SECRET
    def run(args: list[str], *, stage: str, api_key_override: str | None = None) -> subprocess.CompletedProcess[str]:
        global CURRENT_STAGE
        CURRENT_STAGE = stage
        return configured_run(binary, project, args, api_key_override=api_key_override,
                              environment_overrides={"NOLA_CLI_DEBUG_LOG_PATH": str(log_path), "NO_COLOR": "1"})

    assert not log_path.exists(), "debug case requires a fresh owned log"
    marker = "DEBUG_LOG_REAL_7319"
    prompt = f"Reply with exactly {marker} and nothing else."
    forbidden = (secret, "DEBUG_BAD_REQUEST", marker, prompt, os.environ["NOLA_REAL_TEST_API_KEY"])
    with authentication_fault(project, "debug-log", local_secret=secret) as relay:
        failed = run(["--debug", "--json", "--no-tools", "--no-save", "DEBUG_BAD_REQUEST"],
            stage="debug-injected-401", api_key_override=secret)
        relay.assert_injected()
        assert failed.returncode != 0, "injected debug HTTP 401 did not fail"
        assert relay.probe.blocked_secret_requests >= 1, "debug credential did not reach the owned relay"
        failed_log = log_path.read_text(encoding="utf-8")
        failed_audit = audit_debug_log_segment(failed_log, terminal="failed", forbidden=forbidden)
        # JSON no-save output contains journal pointers, not body text. Use the
        # existing human PIPE transport; no persistent seal is claimed here.
        recovered = run(["--debug", "--no-tools", "--no-save", prompt], stage="debug-real-recovery")
        capture_debug_human_answer(project, recovered.stdout, (secret, os.environ["NOLA_REAL_TEST_API_KEY"]))
        assert recovered.returncode == 0, recovered.stderr
        audit_debug_human_answer(recovered.stdout, marker)
        relay.assert_recovered()
    assert log_path.is_file() and log_path.stat().st_size > 0
    log = log_path.read_text(encoding="utf-8")
    assert log.startswith(failed_log), "debug log rewrote the failed invocation"
    recovered_audit = audit_debug_log_segment(log[len(failed_log):], terminal="answered", forbidden=forbidden)
    print(
        json.dumps(
            {
                "case": "debug-log-real-request",
                "logCreated": True,
                "providerResponseLogged": True,
                "secretRedacted": True,
                "bodyNotLogged": True,
                "exactHumanAnswer": True,
                "durableSealProven": False,
                "failedDiagnostic": failed_audit,
                "recoveredDiagnostic": recovered_audit,
            }
        )
    )


def audit_local_context_block(database: pathlib.Path, session: str, prompt: str) -> dict:
    """Known-impossible local input must not become a provider recovery test."""
    assert canonical_text_parts(database, session) == [("user", prompt)], "local blocked input changed or fabricated an answer"
    with readonly(database) as connection:
        connection.execute("BEGIN")
        assert audit_runs(connection, session) == {"runCount":1,"outcomes":["blocked"]}
        run, = connection.execute("SELECT id FROM runs WHERE conversation_id=? AND kind='conversation'", (session,)).fetchone()
        value = json.loads(connection.execute("SELECT result_json FROM run_results WHERE run_id=?", (run,)).fetchone()[0])["value"]
        assert value.get("reason", {}).get("code") == "context.input_budget_exceeded", "wrong local budget blocker"
        facts = [json.loads(row[0]) for row in connection.execute(
            "SELECT payload_json FROM run_events WHERE run_id=? AND event_kind='runtime_decision_fact_observed'", (run,))]
        refs = value.get("blocking_fact_refs")
        assert isinstance(refs, list) and len(refs) == 1
        matched = [fact for fact in facts if fact.get("fact", {}).get("fact_id") == refs[0]]
        assert len(matched) == 1 and matched[0].get("turn_id") == run
        assert matched[0]["fact"]["kind"].get("kind") == "blocker"
        assert matched[0]["fact"]["kind"].get("code") == "context.input_budget_exceeded"
        # All durable attempts, not a join against prunable outbox rows.
        for table in ("run_attempts", "effect_delivery_attempts", "host_event_inbox"):
            assert connection.execute(f"SELECT count(*) FROM {table} WHERE run_id=?", (run,)).fetchone()[0] == 0, "local blocked input was dispatched"
        assert connection.execute("SELECT count(*) FROM messages m JOIN message_parts p ON p.message_id=m.id WHERE m.conversation_id=? AND p.kind='compaction'", (session,)).fetchone()[0] == 0
    return {"runKind":"blocked", "reason":"context.input_budget_exceeded",
        "physicalAttempts":0, "canonicalExactInput":True, "inputCharacters":len(prompt)}


def test_context_input_budget_is_blocked(
    binary: pathlib.Path, project: pathlib.Path
) -> None:
    session = f"local-context-budget-{uuid.uuid4().hex}"
    database = project / "nola-e2e.db"
    fact = f"CTX-ORIGINAL-{uuid.uuid4().hex.upper()}"
    character_count = int(
        os.environ.get("NOLA_REAL_CONTEXT_OVERFLOW_CHARACTERS", "3000000")
    )
    # Known-impossible input stays local: a configured window must not erase
    # the actual provider metadata cap or force a request beyond that cap.
    chunks: list[str] = []
    produced = 0
    counter = 0
    while produced < character_count:
        chunk = hashlib.sha256(f"nola-overflow-{counter}".encode()).hexdigest()
        chunks.append(chunk)
        produced += len(chunk)
        counter += 1
    filler = "".join(chunks)[:character_count]
    prompt = (
        f"Canonical source fact: {fact}. After context recovery reply exactly "
        f"OVERFLOW_RECOVERED_7319.\n{filler}\nCanonical source fact: {fact}."
    )
    request_start = provider_request_count()
    started = time.monotonic()
    try:
        first = configured_run(binary, project, ["--json", "--no-tools", "--session", session],
            timeout=420, input_text=prompt, environment_overrides={
                "NOLA_AGENT_CONTEXT_WINDOW_TOKENS": "4000000",
                "NOLA_AGENT_CONTEXT_TOKEN_THRESHOLD": "2000000",
            })
    except subprocess.TimeoutExpired as error:
        raise AssertionError(
            "local context budget validation exceeded 420s; "
            f"provider={latest_provider_failure()}"
        ) from error
    statuses = provider_statuses_since(request_start)
    assert first.returncode == 0, (
        f"local context admission failed: statuses={statuses}; "
        f"provider={latest_provider_failure()}; stderr={first.stderr[-800:]}"
    )
    assert run_outcomes(database, session) == ["blocked"]
    events = [json.loads(line) for line in pathlib.Path(os.environ["NOLA_REAL_TEST_PROVIDER_LOG"]).read_text().splitlines()]
    assert not any(event.get("requestInputPresent") for event in events[request_start:]), "impossible input reached provider generation"
    audit = audit_local_context_block(database, session, prompt)
    print(json.dumps({"case":"local-context-input-budget-blocked", "passed":True,
        "providerStatuses":statuses, "durationSeconds":round(time.monotonic()-started, 3), **audit}))


def requirement_input_refs(database: pathlib.Path, session: str) -> list[list[str]]:
    with readonly(database) as connection:
        rows = connection.execute("SELECT e.payload_json FROM run_events e JOIN runs r ON r.id=e.run_id WHERE r.conversation_id=? AND e.event_kind='requirement_scope_changed' ORDER BY r.created_at_ms,r.id,e.sequence", (session,)).fetchall()
    return [json.loads(raw)["scope"]["source_input_refs"] for (raw,) in rows]


def test_context_overflow_compaction_and_cold_source_lookup(binary: pathlib.Path, project: pathlib.Path) -> None:
    session = f"context-recovery-{uuid.uuid4().hex}"
    database = project / "nola-e2e.db"
    fact = f"CTX-ORIGINAL-{uuid.uuid4().hex.upper()}"
    filler = "".join(hashlib.sha256(f"nola-context-history-{index}".encode()).hexdigest()
        for index in range(1000))
    seed = f"Source fact: {fact}. Keep this source as history and reply exactly SOURCE_ADMITTED_7319.\n{filler}\nSource fact: {fact}."
    current = "Using the admitted prior history, reply exactly OVERFLOW_RECOVERED_7319 and nothing else."
    started = time.monotonic()
    with provider_fault(project, "context-limit", ContextLimitFaultRelay) as relay:
        first = configured_run(binary, project, ["--json", "--no-tools", "--session", session],
            timeout=420, input_text=seed)
        assert first.returncode == 0, first.stderr
        assert run_outcomes(database, session) == ["answered"]
        assert [text for role, text in canonical_text_parts(database, session) if role == "assistant"] == ["SOURCE_ADMITTED_7319"]
        seed_refs = requirement_input_refs(database, session)
        assert len(seed_refs) == 1 and seed_refs[0], "history lacks admitted source identity"
        relay.arm()
        recovered = configured_run(binary, project, ["--json", "--no-tools", "--session", session],
            timeout=420, input_text=current)
        assert recovered.returncode == 0, recovered.stderr
        relay.assert_recovered()
        statuses = [json.loads(line)["status"] for line in relay.probe.log_path.read_text().splitlines()
            if json.loads(line).get("requestInputPresent")]
    assert run_outcomes(database, session) == ["answered", "answered"]
    parts = canonical_text_parts(database, session)
    assert [text for role, text in parts if role == "user"] == [seed, current], "canonical history or current instruction changed"
    assert [text for role, text in parts if role == "assistant"] == ["SOURCE_ADMITTED_7319", "OVERFLOW_RECOVERED_7319"]
    input_refs = requirement_input_refs(database, session)
    assert len(input_refs) == 2 and input_refs[0] == seed_refs[0] and input_refs[1]
    with readonly(database) as connection:
        compaction_count = connection.execute("SELECT count(*) FROM messages m JOIN message_parts p ON p.message_id=m.id WHERE m.conversation_id=? AND p.kind='compaction'", (session,)).fetchone()[0]
        recovery_run = connection.execute("SELECT id FROM runs WHERE conversation_id=? ORDER BY created_at_ms DESC,id DESC LIMIT 1", (session,)).fetchone()[0]
        failures = [json.loads(raw) for (raw,) in connection.execute(
            "SELECT payload_json FROM host_event_inbox WHERE run_id=?", (recovery_run,))
            if json.loads(raw).get("payload", {}).get("type") == "failed"]
        assert len(failures) == 1, "injected context error lacks a unique accepted Host observation"
        failure = failures[0]
        assert failure.get("session_id") == session and failure.get("turn_id") == recovery_run
        assert failure["payload"]["error"].get("code") == "context_length_exceeded"
        assert failure["payload"]["error"].get("http_status") == 400
    assert compaction_count >= 1
    # A distinct CLI process reads actual canonical lineage, not the model's
    # generated summary. Full bodies were compared independently above.
    sources = run_process(
        binary,
        project,
        ["session", "sources", session],
        stage="cold-compaction-source-lookup",
    )
    assert sources.returncode == 0, sources.stderr
    source_payload = json.loads(sources.stdout)
    source_diagnostic = {
        "missing_source_item_ids": source_payload.get("missing_source_item_ids"),
        "source_count": len(source_payload.get("sources", [])),
        "source_item_ids": source_payload.get("source_item_ids"),
        "source_bodies_are_previews": source_payload.get("source_bodies_are_previews"),
    }
    assert source_payload.get("missing_source_item_ids") == [], source_diagnostic
    assert source_payload.get("source_bodies_are_previews") is True, source_diagnostic
    assert fact in json.dumps(source_payload, ensure_ascii=False), source_diagnostic
    assert source_payload.get("source_fingerprint"), source_diagnostic
    assert set(seed_refs[0]).issubset(source_payload.get("source_item_ids", [])), "cold lineage lost original source identity"
    assert set(input_refs[1]).isdisjoint(source_payload["source_item_ids"]), "current user instruction was summarized"
    print(
        json.dumps(
            {
                "case": "context-overflow-cold-source-lookup",
                "passed": True,
                "injectedContextLimit": True,
                "naturalProviderOverflowProven": False,
                "historyCharacters": len(seed),
                "providerStatuses": statuses,
                "compactionParts": compaction_count,
                "originalSourceRefs": seed_refs[0],
                "currentSourceRefs": input_refs[1],
                "canonicalExactInputs": True,
                "acceptedFailureRunID": recovery_run,
                "durationSeconds": round(time.monotonic() - started, 3),
            }
        )
    )


def main() -> None:
    global CURRENT_STAGE
    global ORACLE_BINARY
    parser = argparse.ArgumentParser()
    parser.add_argument("binary", type=pathlib.Path)
    parser.add_argument("oracle", type=pathlib.Path)
    args = parser.parse_args()
    binary = args.binary.expanduser().resolve()
    if not binary.is_file():
        raise SystemExit(f"CLI binary not found: {binary}")
    ORACLE_BINARY = args.oracle.expanduser().resolve()
    if not ORACLE_BINARY.is_file():
        raise SystemExit(f"terminal acceptance oracle not found: {ORACLE_BINARY}")

    configured_project = os.environ.get("NOLA_REAL_RESILIENCE_PROJECT", "").strip()
    directory = configured_project or tempfile.mkdtemp(prefix="nola-real-resilience-")
    with retained_project(directory) as project:
        CURRENT_STAGE = "provider-ready"
        test_provider_ready(binary, project)
        only_case = os.environ.get("NOLA_REAL_TEST_ONLY_CASE", "").strip()
        if only_case:
            targeted_cases = {
                "queued-follow-up": test_queued_follow_up,
                "permission-custom-response": test_permission_custom_response,
                "allow-once-prompts-again": test_allow_once_prompts_again,
                "deny-no-substitution": test_deny_has_no_side_effect_or_tool_substitution,
                "read-only-no-permission": test_read_only_inspection_needs_no_permission,
                "external-workspace-access-followup": test_external_workspace_access_and_followup,
                "complete-diff-discovery-followup": test_complete_diff_discovery_and_natural_followup,
                "multilingual-ambiguous-no-forced-tools": test_multilingual_ambiguous_words_do_not_force_tools,
                "goal-return-to-composer-stops-successors": test_goal_return_to_composer_stops_successors,
                "session-permission-restart": test_session_permission_survives_restart,
                "context-overflow-cold-source-lookup": test_context_overflow_compaction_and_cold_source_lookup,
                "local-context-input-budget-blocked": test_context_input_budget_is_blocked,
                "debug-log-real-request": test_debug_log_real_request,
            }
            selected = targeted_cases.get(only_case)
            if selected is None:
                raise AssertionError(f"unknown targeted real-provider case: {only_case}")
            CURRENT_STAGE = only_case
            selected(binary, project)
            return
        if os.environ.get("NOLA_REAL_TEST_AUTH_FAILURE", "").strip() == "1":
            CURRENT_STAGE = "auth-failure-recovery"
            test_auth_failure_and_recovery(binary, project)
        for scenario in (test_queued_follow_up, test_permission_custom_response,
                test_allow_once_prompts_again, test_deny_has_no_side_effect_or_tool_substitution,
                test_read_only_inspection_needs_no_permission, test_multilingual_ambiguous_words_do_not_force_tools,
                test_goal_return_to_composer_stops_successors, test_session_permission_survives_restart,
                test_failed_turn_session_recovery, test_escape_records_cancelled_turn, test_debug_log_real_request):
            CURRENT_STAGE = scenario.__name__
            scenario(binary, project)


if __name__ == "__main__":
    def interrupted(signum, _frame):
        raise RuntimeError(f"resilience validation interrupted by signal {signum}")
    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGINT, interrupted)
    try:
        main()
    except (AssertionError, RuntimeError, ValueError, OSError, subprocess.TimeoutExpired) as error:
        print(json.dumps(sanitized_failure(error)), file=sys.stderr)
        raise SystemExit(1) from None
