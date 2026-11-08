# SPDX-License-Identifier: Apache-2.0
"""Cold, read-only corroboration of the Mac harness's exact overwrite decision."""
from __future__ import annotations

import argparse
import base64
import hashlib
import json
from pathlib import Path
import sqlite3
import tempfile

from canonical_run_audit import audit_runs


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def tool_execution(connection: sqlite3.Connection, events: list, run: str,
                   conversation: str, assistant: str, call: str, capability: str,
                   expected_count: int) -> dict:
    """Cross-check current Part diagnostics against durable Host/physical facts.

    Part previews are bounded diagnostics, not complete tool input or output.
    Actual execution/output authority comes from execution_evidence_sources and
    the exact Host response and delivery attempt they identify.
    """
    parts = [(seq, event) for seq, kind, event in events
             if kind == "part_updated" and event.get("part", {}).get("kind") == "tool"
             and event["part"].get("call_id") == call]
    for _, event in parts:
        part, update = event["part"], event["update"]
        require(event["type"] == "part_updated" and event["diagnostic_schema"] == "nola.part_event.v1"
                and event["turn_id"] == part["turn_id"] == run
                and part["message_id"] == assistant and part["capability_id"] == capability
                and update["part_id"] == part["id"], "tool Part owner/identity mismatch")
    require(len({event["part"]["id"] for _, event in parts}) <= 1, "tool call has multiple Part identities")
    starts = [(seq, event) for seq, event in parts
              if event["update"]["type"] == "tool_state" and event["update"]["state"] == "running"]
    require(len(starts) == expected_count, "denied action executed or approved action replayed")
    require(all(event["part"]["state"] == "running" for _, event in starts), "tool state update disagrees with its Part")
    completions = [(effect, request, event_id, json.loads(raw)) for effect, request, event_id, raw in connection.execute(
        "SELECT effect_id,request_id,event_id,payload_json FROM host_event_inbox WHERE run_id=?", (run,))
        if json.loads(raw)["payload"].get("type") == "tool_completed"
        and json.loads(raw)["payload"].get("call_id") == call]
    evidence = [(seq, event) for seq, kind, event in events
                if kind == "execution_evidence_observed" and event["evidence"]["call_id"] == call]
    require(len(completions) == len(evidence) == expected_count,
            "Host completion and canonical execution evidence count disagree with the decision")
    effects = {effect for effect, _, _, _ in completions}
    for effect, raw in connection.execute("""SELECT e.effect_id,p.payload_json FROM effect_outbox e
            JOIN effect_outbox_payloads p ON p.effect_id=e.effect_id WHERE e.run_id=?""", (run,)):
        payload = json.loads(raw)["payload"]
        if payload.get("type") == "execute_tool" and payload.get("call_id") == call:
            effects.add(effect)
    attempts = [(attempt, effect, request, state, json.loads(raw)) for attempt, effect, request, state, raw in connection.execute(
        "SELECT id,effect_id,request_id,state,record_json FROM effect_delivery_attempts WHERE run_id=?", (run,))
        if effect in effects]
    physical = [row for row in attempts if row[4].get("execution_started_at_ms") is not None]
    require(len(physical) == expected_count
            and all(state == record["state"] == "settled" for _, _, _, state, record in attempts),
            "physical delivery count or unsettled execution disagrees with the decision")
    if not expected_count:
        return {"starts": [], "parts": parts, "physical": [], "completion": None, "evidence_sequence": None}
    require(expected_count == 1, "acceptance only admits one execution per call")
    effect, request, event_id, envelope = completions[0]
    response = envelope["payload"]
    attempt_id, physical_effect, physical_request, _, record = physical[0]
    require(envelope["effect_id"] == effect == physical_effect == record["effect_id"]
            and envelope["request_id"] == request == physical_request == record["request_id"]
            and envelope["turn_id"] == record["run_id"] == run
            and envelope["session_id"] == conversation
            and envelope["delivery"]["attempt_id"] == attempt_id == record["id"]
            and response["outcome"] == record["settlement"] == "succeeded",
            "Host completion is not bound to its successful physical delivery")
    for key in ("worker_generation", "lease_owner", "lease_token"):
        require(envelope["delivery"][key] == record[key], "Host completion delivery fence mismatch")
    sequence, fact = evidence[0]
    source, value = fact["evidence"]["source"], fact["evidence"]
    require(fact["type"] == "execution_evidence_observed" and fact["turn_id"] == run
            and fact["core_envelope"]["session_id"] == conversation
            and fact["core_envelope"]["turn_id"] == run
            and value["tool_name"] == capability and value["outcome"] == response["outcome"]
            and source["effect_id"] == effect and source["request_id"] == request
            and source["host_id"] == envelope["host_id"]
            and source["response_sequence"] == str(envelope["sequence"])
            and source["catalog_revision"] == str(envelope["catalog_revision"])
            and source["availability_epoch"] == str(envelope["availability_epoch"]),
            "canonical execution evidence differs from the exact Host response")
    binding = connection.execute("""SELECT s.conversation_id,s.turn_id,s.call_id,s.core_session_id,s.host_event_id,
        s.core_revision,s.core_cursor,e.sequence,e.payload_json FROM execution_evidence_sources s
        JOIN run_events e ON e.event_id=s.source_event_id AND e.run_id=s.run_id
        WHERE s.run_id=? AND s.evidence_id=?""", (run, value["evidence_id"])).fetchone()
    require(binding is not None and tuple(binding[:5]) == (conversation, run, call, conversation, event_id)
            and binding[5] == fact["core_envelope"]["revision"]
            and binding[6] == fact["core_envelope"]["cursor"]["sequence"]
            and binding[7] == sequence and json.loads(binding[8]) == fact,
            "execution evidence lacks its durable Host source binding")
    output = response["output"]
    require(isinstance(output, dict) and output.get("contract") == "nola.tool_result.v2"
            and "execution" in output, "current ToolResult V2 with explicit execution is required")
    execution = output["execution"]
    # This audit covers direct file writes/reads, not an owned OS process. It
    # cannot manufacture or authenticate a separate local-process stop proof.
    require(execution is None or (isinstance(execution, dict)
            and "local_stop" in execution and execution["local_stop"] is None),
            "file execution receipt requires explicit local_stop null; no local stop is proven")
    require(value["receipt"] == execution,
            "canonical execution receipt differs from the actual tool result")
    done = [(seq, event) for seq, event in parts
            if event["update"]["type"] == "tool_state" and event["update"]["state"] == "completed"]
    require(len(done) == 1 and starts[0][0] < sequence < done[0][0]
            and done[0][1]["part"]["state"] == "completed", "missing ordered successful tool Part lifecycle")
    return {"starts": starts, "parts": parts, "physical": physical,
            "completion": envelope, "evidence_sequence": sequence, "completed_sequence": done[0][0]}


def audit(connection: sqlite3.Connection, presentation: dict, workspace: Path,
          decision: str, original: bytes, expected: bytes) -> dict:
    observations = presentation.get("permission_audits", [])
    require(len(observations) == 1, "exactly one pre-decision audit is required")
    observation = observations[0]
    fixture = presentation["permission_fixture"]
    target = (workspace / fixture["relative_path"]).resolve(strict=True)
    require(target.is_relative_to(workspace.resolve()), "fixture escaped its owned workspace")
    require(observation["phase"] == "before_permission_decision"
            and observation["canonical_pending_verified"] is True
            and observation["fixture_unchanged"] is True, "missing canonical pre-decision witness")
    digest = hashlib.sha256(original).hexdigest()
    require(base64.b64decode(observation["fixture_bytes_base64"], validate=True) == original
            and base64.b64decode(fixture["original_bytes_base64"], validate=True) == original
            and observation["fixture_sha256"] == observation["original_sha256"] == fixture["original_sha256"] == digest,
            "original bytes/hash were not intact immediately before resolution")
    require(target.read_bytes() == expected
            and base64.b64decode(fixture["final_bytes_base64"], validate=True) == expected
            and fixture["final_sha256"] == hashlib.sha256(expected).hexdigest(), "final bytes differ from the independent expectation")
    conversation, run, assistant = (presentation[key] for key in
                                    ("conversation_id", "run_id", "assistant_message_id"))
    require(all(observation[key] == value for key, value in
                (("conversation_id", conversation), ("run_id", run), ("assistant_message_id", assistant))),
            "pre-decision witness has another owner")
    subject = json.loads(observation["resource"])
    require(subject["capability"] == observation["capability_id"] and subject["operation"] == "filesystem"
            and any(resource.get("kind") == "file" and resource.get("access") == "write"
                    and "/" + "/".join(resource["components"]) == str(target)
                    for resource in subject["resources"]), "pending resource is not the exact fixture overwrite")
    canonical = audit_runs(connection, conversation)
    expected_kind = "action_completed" if decision == "allow_once" else "blocked"
    require(canonical == {"runCount": 1, "outcomes": [expected_kind]}, "cold canonical seal/result differs from the scenario")
    owner = connection.execute("SELECT assistant_message_id,status FROM runs WHERE id=? AND conversation_id=?",
                               (run, conversation)).fetchone()
    require(owner is not None and tuple(owner) == (assistant, "sealed"), "cold Run owner/status mismatch")
    events = [(sequence, kind, json.loads(raw)) for sequence, kind, raw in connection.execute(
        "SELECT sequence,event_kind,payload_json FROM run_events WHERE run_id=? ORDER BY sequence", (run,))]
    call = observation["call_id"]
    expected_executions = int(decision == "allow_once")
    execution = tool_execution(connection, events, run, conversation, assistant, call,
                               observation["capability_id"], expected_executions)
    permissions = [(seq, event["update"]["permission"]) for seq, event in execution["parts"]
                   if event["update"]["type"] == "tool_permission"]
    requested = [(seq, value) for seq, value in permissions if value["decision"] is None]
    resolved = [(seq, value) for seq, value in permissions if value["decision"] is not None]
    settlements = [(seq, event) for seq, kind, event in events if kind == "permission_settled"]
    require(len(requested) == len(resolved) == len(settlements) == 1,
            "duplicate or missing canonical permission lifecycle")
    settled_sequence, settled = settlements[0]
    receipt = settled["receipt"]
    require(settled["type"] == "permission_settled"
            and all(value["request_id"] == observation["request_id"] for _, value in permissions)
            and requested[0][0] < settled_sequence < resolved[0][0]
            and resolved[0][1]["decision"] == observation["decision"] == decision
            and receipt == {"request_id": observation["request_id"], "turn_id": run,
                "call_id": call, "capability_id": observation["capability_id"], "resource": observation["resource"],
                "outcome": {"type": "authorization", "decision": decision}}
            and settled["core_envelope"]["session_id"] == conversation
            and settled["core_envelope"]["turn_id"] == run,
            "canonical request/decision identity or order mismatch")
    responses = [json.loads(raw) for (raw,) in connection.execute(
        "SELECT payload_json FROM host_event_inbox WHERE run_id=?", (run,))
        if json.loads(raw)["payload"].get("type") == "permission_resolved"]
    require(len(responses) == 1 and responses[0]["session_id"] == conversation
            and responses[0]["turn_id"] == run
            and responses[0]["payload"] == {"type": "permission_resolved", "request_id": observation["request_id"],
                "resolution": {"type": "authorization", "decision": decision}},
            "canonical permission settlement lacks its exact Host authorization response")
    if execution["starts"]:
        require(execution["starts"][0][0] > resolved[0][0], "tool started before its canonical approval")
    # A different call ID must not conceal a second physical overwrite of the
    # same fixture. This is an exact bound capability/path check, not routing.
    other_writes = [event["part"]["call_id"] for _, kind, event in events
        if kind == "part_updated" and event["part"].get("kind") == "tool"
        and event["part"].get("capability_id") == observation["capability_id"]
        and event["part"].get("call_id") != call and event["part"].get("started_at_ms") is not None
        and isinstance(event["part"].get("input", {}).get("path"), str)
        and (workspace / event["part"]["input"]["path"]).resolve() == target]
    require(not other_writes, "another call executed the same fixture overwrite")
    writes = [(event["evidence"]["call_id"], effect) for _, kind, event in events
        if kind == "execution_evidence_observed" and isinstance(event["evidence"].get("receipt"), dict)
        for effect in event["evidence"]["receipt"].get("effects", [])
        if effect.get("resource_uri") == target.as_uri() and effect.get("operation") == "write"]
    require(len(writes) == expected_executions and all(owner == call for owner, _ in writes),
            "another or duplicate execution changed the fixture")
    if decision == "allow_once":
        output = execution["completion"]["payload"]["output"]
        require(output["execution"]["state"] == "completed" and output["execution"]["effects"] == [
            {"operation": "write", "resource_uri": target.as_uri(), "before_version": "sha256:" + digest,
             "after_version": "sha256:" + hashlib.sha256(expected).hexdigest()}],
            "actual overwrite receipt does not match the exact fixture versions")
        later_calls = {event["part"]["call_id"] for seq, kind, event in events
                       if kind == "part_updated" and event["part"].get("kind") == "tool"
                       and event["part"].get("capability_id") == "read_file"
                       and event["update"]["type"] == "tool_state" and event["update"]["state"] == "running"
                       and seq > execution["completed_sequence"]
                       and isinstance(event["part"].get("input", {}).get("path"), str)
                       and (workspace / event["part"]["input"]["path"]).resolve() == target}
        require(bool(later_calls), "no real post-overwrite read-back evidence")
        for read_call in later_calls:
            read = tool_execution(connection, events, run, conversation, assistant, read_call, "read_file", 1)
            # These wrappers own a one-line fixture. Check the complete current
            # read_file rendering, not a marker mentioned in metadata or prose.
            text = read["completion"]["payload"]["output"]["presentation"]["text"]
            numbered = read["starts"][0][1]["part"]["input"].get("show_line_numbers") is not False
            body = "   1 | " + expected.decode("utf-8").rstrip("\n") if numbered else expected.decode("utf-8")
            require(text == f"# {target}\nlines: 1-1 of 1\n\n{body}",
                    "post-overwrite Host read-back did not contain the expected bytes")
    return {"passed": True, "conversation_id": conversation, "run_id": run,
            "request_id": observation["request_id"], "call_id": call, "decision": decision,
            "canonical_result": expected_kind, "canonical_tool_starts": len(execution["starts"]),
            "host_completions": expected_executions, "physical_executions": len(execution["physical"]),
            "original_intact_before_decision": True, "final_fixture_verified": True}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--workspace", type=Path, required=True)
    parser.add_argument("--decision", choices=("allow_once", "deny"), required=True)
    parser.add_argument("--original", required=True)
    parser.add_argument("--expected", required=True)
    args = parser.parse_args()
    root = args.workspace.resolve(strict=True)
    temporary_roots = (Path("/tmp").resolve(), Path(tempfile.gettempdir()).resolve())
    if not root.name.startswith("nola-mac-") or not any(root != base and root.is_relative_to(base) for base in temporary_roots):
        parser.error("workspace must be an owned nola-mac-* temporary fixture directory")
    report = root / "permission-fixture-audit.json"
    try:
        presentation = json.loads((root / "result.json").read_text())
        database = root / "ApplicationSupport/Nola/State/nola-v4.db"
        with sqlite3.connect(database.as_uri() + "?mode=ro", uri=True) as connection:
            connection.execute("BEGIN")
            result = audit(connection, presentation, root, args.decision,
                           args.original.encode(), args.expected.encode())
    except (ValueError, KeyError, TypeError, OSError, sqlite3.Error) as error:
        result = {"passed": False, "error": str(error)}
    report.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result))
    return 0 if result["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
