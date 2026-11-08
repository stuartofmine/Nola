#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Real source HTTP recovery bound to a separate client's durable Room intent.

No builds, physical devices, user-history imports or source-config writes.
The two databases and all processes belong to mktemp fixtures. Model output is
real LAN inference; Run/Attempt frontiers are independent read-only SQL oracles.
"""
import argparse
import json
import os
from pathlib import Path
import sqlite3
import tempfile
import time
import uuid
import urllib.error
import urllib.request
from urllib.parse import urlsplit

from test_real_submission_journal import Host, require
from test_real_agent_room_isolation import command, room_command, start_service, stop_service
from e2e.real_overwrite_permission import isolated_environment, scrub_case_credentials


def now():
    return int(time.time() * 1000)


def run_count(host):
    with sqlite3.connect((host.directory / "acceptance.sqlite").as_uri() + "?mode=ro", uri=True) as db:
        return db.execute("SELECT COUNT(*) FROM runs").fetchone()[0]


def stream_outcome(host, body):
    request = urllib.request.Request(host.base + "/api/chat/agent/v2/stream",
        data=json.dumps(body).encode(), headers={"Content-Type": "application/json"})
    try:
        response = host.http.open(request, timeout=240)
    except urllib.error.HTTPError as error:
        with error:
            failure = json.load(error)
        return error.code, [], failure.get("code")
    events = []
    with response:
        for line in response:
            if line.startswith(b"data:") and line[5:].strip() != b"[DONE]":
                events.append(json.loads(line[5:]))
    return 200, events, None


def freeze(host, sock, label, text, context_window):
    room = room_command(sock, type="create", title=label, at_ms=now())
    room = room_command(sock, type="join", input=dict(room_id=room["room"]["id"],
        expected_room_revision=room["revision"], agent_id="transport-agent", expected_agent_revision=1, at_ms=now()))
    participant = room["room"]["participants"][0]
    context = participant["context_conversation_id"]
    execution = str(uuid.uuid4())
    identity = dict(source_host_id=host.host_id, client_id="real-room-transport", conversation_id=context,
        submission_id=execution, user_message_id=execution + ":user", assistant_message_id=execution + ":assistant")
    plan = host.operation("/api/agent/tool-plan/prepare", dict(expected_host_id=host.host_id,
        client_id=identity["client_id"], session_id=context))
    body = dict(conversationId=context, expected_host_id=host.host_id, client_id=identity["client_id"],
        submission_id=execution, user_message_id=identity["user_message_id"],
        assistant_message_id=identity["assistant_message_id"], execution_backend="nola",
        tool_plan_ref=plan["reference"], messages=[dict(role="user", content=text)], max_iterations=4,
        max_context_tokens=context_window, max_output_tokens=256)
    prepared = host.operation("/api/chat/agent/v2/submission", dict(operation="prepare", identity=identity, request=body))
    body["request_digest"] = prepared["request_digest"]
    submission = room_command(sock, type="submit", input=dict(room_id=room["room"]["id"],
        submission_id=identity["submission_id"], expected_room_revision=room["revision"],
        text=text, participant_id=participant["id"], at_ms=now()))
    require(submission["state"] == "selected", "Explicit recipient was not selected")
    command(sock, dict(type="prepare_remote_submission", input=dict(
        source_host_id=identity["source_host_id"], client_id=identity["client_id"],
        conversation_id=context, conversation_title=label, submission_id=identity["submission_id"],
        user_message_id=identity["user_message_id"], user_part_id=identity["user_message_id"] + ":text",
        user_text=text, user_content_json=None, assistant_message_id=identity["assistant_message_id"],
        assistant_part_id=identity["assistant_message_id"] + ":text", context_checkpoint_id=None, created_at_ms=now())))
    semantic = {key: value for key, value in body.items() if key != "request_digest"}
    bound = command(sock, dict(type="bind_remote_submission_request", identity=identity, request=semantic))
    require(bound["phase"] == "bound" and bound["request_digest"] == body["request_digest"],
        "Client and source did not freeze the same semantic request")
    intent = dict(execution_id=identity["submission_id"], envelope=dict(kind="remote_host",
        identity=identity, request_digest=body["request_digest"]))
    submission = room_command(sock, type="prepare_execution", room_id=room["room"]["id"],
        submission_id=identity["submission_id"], expected_revision=submission["revision"], intent=intent, at_ms=now())
    return room["room"]["id"], submission, intent


def original_body(sock, intent):
    envelope = intent["envelope"]
    body = command(sock, dict(type="read_remote_submission_request", identity=envelope["identity"],
        request_digest=envelope["request_digest"]))
    body["request_digest"] = envelope["request_digest"]
    return body


def read_intent(sock, room, submission):
    return room_command(sock, type="execution_intent", room_id=room,
        submission_id=submission["input"]["submission_id"])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--service", type=Path, required=True)
    parser.add_argument("--config", type=Path, required=True)
    args = parser.parse_args()
    require(os.environ.get("NOLA_RUN_LIVE_ROOM_TRANSPORT_E2E") == "1", "Explicit live acceptance opt-in required")
    require(__debug__, "Do not run acceptance with Python -O")
    llm = json.loads(args.config.read_text())["llm"]
    endpoint = urlsplit(llm["endpoint"])
    require(endpoint.hostname == "192.168.31.18" and endpoint.port == 8081,
        "Acceptance requires the authorized LAN model endpoint")
    catalog_request = urllib.request.Request(llm["endpoint"].rstrip("/") + "/models",
        headers={"Authorization": "Bearer " + llm["api_key"]})
    with urllib.request.urlopen(catalog_request, timeout=10) as response:
        catalog = json.load(response)
    model = next(item for item in catalog["data"] if item["id"] == llm["model"])
    context_window = model["meta"]["n_ctx"]
    require(type(context_window) is int and 512 <= context_window <= 4_000_000, "Invalid source model context declaration")
    root = Path(tempfile.mkdtemp(prefix="nola-room-transport-"))
    environment = isolated_environment(root, "cautious")
    service = host = None
    evidence = dict(passed=False, client_fixture=str(root), real_lan_model_configured=True,
        real_inference_verified=False, model_context_window=context_window,
        physical_provider_http_calls_observed=False, apple_ui_projection_verified=False,
        transport="loopback source HTTP (not encrypted mobile bridge)")
    print("Isolated client evidence:", root, flush=True)
    try:
        host = Host(args.binary.resolve(strict=True), args.config.resolve(strict=True))
        evidence["source_fixture"] = str(host.directory)
        service, sock = start_service(args.service.resolve(strict=True), root, environment)
        directory = room_command(sock, type="directory")
        room_command(sock, type="configure_agent", input=dict(id="transport-agent", name="Transport acceptance",
            description="Isolated transport fixture", expected_directory_revision=directory["revision"],
            expected_agent_revision=None, defaults={}, availability="active", updated_at_ms=now()))

        room, cancelled, intent = freeze(host, sock, "Cancel before claim", "This cancelled request must never execute.", context_window)
        late_body = original_body(sock, intent)  # Already captured transport can arrive after cancellation.
        cancelled = room_command(sock, type="cancel_pending", room_id=room,
            submission_id=cancelled["input"]["submission_id"], expected_revision=cancelled["revision"], at_ms=now())
        envelope = intent["envelope"]
        tombstone = host.operation("/api/chat/agent/v2/submission", dict(operation="cancel",
            identity=envelope["identity"], request_digest=envelope["request_digest"]))
        require(tombstone["phase"] == "cancelled", "Source did not persist cancellation tombstone")
        command(sock, dict(type="accept_remote_submission_receipt", receipt=tombstone))
        source_id = host.host_id
        stop_service(service)
        service = None
        host.stop()
        host.start()
        service, sock = start_service(args.service.resolve(), root, environment)
        require(host.host_id == source_id and read_intent(sock, room, cancelled) == intent, "Restart changed frozen ownership")
        late_status, late, _ = stream_outcome(host, late_body)
        cold_tombstone = host.operation("/api/chat/agent/v2/submission", dict(operation="observe",
            identity=envelope["identity"], request_digest=envelope["request_digest"]))
        require(cold_tombstone == tombstone and run_count(host) == 0
            and (late_status == 409 or len(late) == 1 and late[0].get("receipt") == tombstone),
            "Late POST bypassed durable cancellation or created a Run")
        evidence["prepared_cancel_after_restart"] = dict(tombstone=tombstone, source_run_count=0, late_http_status=late_status)
        print("PASS: prepared Room cancellation survives source/client restart; late original POST creates no Run", flush=True)

        marker = "ROOM_TRANSPORT_" + uuid.uuid4().hex[:12]
        room, selected, intent = freeze(host, sock, "Claim before transport", "Reply only " + marker + ". Do not use tools.", context_window)
        claim = room_command(sock, type="claim_execution", room_id=room,
            submission_id=selected["input"]["submission_id"], expected_revision=selected["revision"], at_ms=now())
        require(claim["dispatch_permitted"] and claim["submission"]["state"] == "executing", "First claim did not authorize dispatch")
        stop_service(service)
        service = None
        host.stop()
        host.start()
        service, sock = start_service(args.service.resolve(), root, environment)
        recovered = read_intent(sock, room, selected)
        require(recovered == intent and run_count(host) == 0, "Pre-admission restart changed intent or fabricated Run")
        repeated_claim = room_command(sock, type="claim_execution", room_id=room,
            submission_id=selected["input"]["submission_id"], expected_revision=claim["submission"]["revision"], at_ms=now())
        require(not repeated_claim["dispatch_permitted"], "Recovery minted another dispatch permission")
        envelope = recovered["envelope"]
        observed = host.operation("/api/chat/agent/v2/submission", dict(operation="observe",
            identity=envelope["identity"], request_digest=envelope["request_digest"]))
        require(observed is None, "Undispatched request unexpectedly has a source admission")
        body = original_body(sock, recovered)
        status, events, code = stream_outcome(host, body)
        evidence["cold_first_dispatch"] = dict(http_status=status, code=code, source_run_count=run_count(host))
        require(status == 200, "Cold original request was rejected: " + str(code))
        require(any(event.get("receipt", {}).get("phase") == "admitted" for event in events), "Original request was not admitted")
        seal, manifest = host.completed(envelope["identity"], envelope["request_digest"])
        require(marker in host.text(manifest), "Real LAN response did not contain requested marker")
        evidence["real_inference_verified"] = True
        frontier = host.attempt_frontier(envelope["identity"])
        require(run_count(host) == 1, "Expected exactly one canonical Run")
        for restart in (False, True):
            if restart:
                host.stop()
                host.start()
            repeated = host.stream(original_body(sock, recovered))
            require(repeated and all(event.get("type") == "submission_receipt" for event in repeated), "Duplicate executed a second stream")
            require(host.completed(envelope["identity"], envelope["request_digest"]) == (seal, manifest), "Duplicate changed terminal seal")
            require(run_count(host) == 1 and host.attempt_frontier(envelope["identity"]) == frontier,
                "Duplicate/restart created a Run or changed physical Attempt frontier")
        evidence.update(claimed_before_transport_restart=True, duplicate_post_before_and_after_restart=True,
            original_intent=recovered, source_run_count=1, durable_attempt_frontier=frontier, terminal_seal=seal)
        print("PASS: frozen claim survives cold restart; repeated original HTTP POST preserves one Run and Attempt frontier", flush=True)

        room, selected, rejected_intent = freeze(host, sock, "Catalog changed after claim",
            "This frozen plan must be rejected after the source catalog changes.", context_window)
        claim = room_command(sock, type="claim_execution", room_id=room,
            submission_id=selected["input"]["submission_id"], expected_revision=selected["revision"], at_ms=now())
        require(claim["dispatch_permitted"], "Rejection fixture did not claim")
        # Use the real settings endpoint, never rewrite plan storage or clocks.
        host.json("/api/agent/settings", {"host_tools_enabled": False})
        status, _, code = stream_outcome(host, original_body(sock, rejected_intent))
        require(status == 409 and code == "host_tool_plan_rejected", "Changed source catalog was not rejected")
        envelope = rejected_intent["envelope"]
        rejected = host.operation("/api/chat/agent/v2/submission", dict(operation="observe",
            identity=envelope["identity"], request_digest=envelope["request_digest"]))
        require(rejected is not None and rejected["phase"] == "rejected", "Source rejection was not durable")
        command(sock, dict(type="accept_remote_submission_receipt", receipt=rejected))
        reconciled = room_command(sock, type="reconcile_execution", room_id=room,
            submission_id=selected["input"]["submission_id"], at_ms=now())
        evidence["changed_catalog_rejection"] = dict(receipt=rejected,
            room_state=reconciled["state"], source_run_count=run_count(host))
        require(reconciled["state"] == "failed" and run_count(host) == 1,
            "Rejected request remained executing or created another Run")
        evidence["passed"] = True
        print("PASS: changed frozen catalog persists source rejection; client Room reconciles failed without another Run", flush=True)
    finally:
        if service is not None:
            stop_service(service)
        if host is not None:
            host.close()
            # Retain logs/DB evidence but remove this fixture's credential vault.
            scrub_case_credentials(host.directory)
        scrub_case_credentials(root)
        with (root / "evidence.json").open("x") as output:
            os.chmod(root / "evidence.json", 0o600)
            json.dump(evidence, output, ensure_ascii=False, indent=2)
        print("Evidence:", root / "evidence.json", flush=True)


if __name__ == "__main__":
    main()
