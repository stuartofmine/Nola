# SPDX-License-Identifier: Apache-2.0
"""Current Store/Host wire fixtures, not real-model acceptance evidence.

Shapes follow the current ToolResult V2 contract: bounded part_updated diagnostics,
execution_evidence_observed, Host envelopes, and physical delivery records.
No invented run_events.tool_started/tool_completed/permission_resolved events.
"""
import base64
import hashlib
import json
import sqlite3

import pytest
import audit_mac_permission_fixture as auditor


def fixture(tmp_path, decision):
    original, expected = b"owned original\n", b"approved\n" if decision == "allow_once" else b"owned original\n"
    target = tmp_path / "existing.txt"
    target.write_bytes(expected)
    digest = lambda value: hashlib.sha256(value).hexdigest()
    b64 = lambda value: base64.b64encode(value).decode()
    resource = json.dumps({"execution_target": "fixture-host", "capability": "write_file", "operation": "filesystem",
        "resources": [{"kind": "file", "filesystem_id": "fixture-filesystem", "access": "write",
                       "components": list(target.parts[1:])}]})
    observation = {"phase": "before_permission_decision", "canonical_pending_verified": True,
        "fixture_unchanged": True, "fixture_bytes_base64": b64(original),
        "fixture_sha256": digest(original), "original_sha256": digest(original),
        "conversation_id": "conversation", "run_id": "run", "assistant_message_id": "assistant",
        "request_id": "permission", "call_id": "write", "capability_id": "write_file", "decision": decision,
        "resource": resource}
    presentation = {"conversation_id": "conversation", "run_id": "run", "assistant_message_id": "assistant",
        "permission_audits": [observation], "permission_fixture": {"relative_path": target.name,
            "original_bytes_base64": b64(original), "original_sha256": digest(original),
            "final_bytes_base64": b64(expected), "final_sha256": digest(expected)}}
    connection = sqlite3.connect(":memory:")
    connection.executescript("""
        CREATE TABLE runs(id TEXT,conversation_id TEXT,kind TEXT,status TEXT,
            assistant_message_id TEXT,completed_at_ms INTEGER,created_at_ms INTEGER);
        CREATE TABLE assistant_run_terminal_seals(run_id TEXT,conversation_id TEXT,seal_json TEXT);
        CREATE TABLE run_results(run_id TEXT,result_json TEXT);
        CREATE TABLE run_events(run_id TEXT,sequence INTEGER,event_kind TEXT,payload_json TEXT,event_id TEXT);
        CREATE TABLE host_event_inbox(run_id TEXT,effect_id TEXT,request_id TEXT,event_id TEXT,payload_json TEXT);
        CREATE TABLE effect_outbox(run_id TEXT,effect_id TEXT);
        CREATE TABLE effect_outbox_payloads(effect_id TEXT,payload_json TEXT);
        CREATE TABLE effect_delivery_attempts(id TEXT,run_id TEXT,effect_id TEXT,request_id TEXT,state TEXT,record_json TEXT);
        CREATE TABLE execution_evidence_sources(run_id TEXT,evidence_id TEXT,conversation_id TEXT,
            turn_id TEXT,call_id TEXT,core_session_id TEXT,host_event_id TEXT,
            core_revision INTEGER,core_cursor INTEGER,source_event_id TEXT);
    """)
    kind = "action_completed" if decision == "allow_once" else "blocked"
    result = {"result_id": "result", "run_id": "run", "conversation_id": "conversation", "value": {"kind": kind}}
    seal = {"identity": {"run_id": "run", "conversation_id": "conversation", "assistant_message_id": "assistant"},
            "completed_at_ms": 100, "result": result}
    connection.execute("INSERT INTO runs VALUES ('run','conversation','conversation','sealed','assistant',100,1)")
    connection.execute("INSERT INTO assistant_run_terminal_seals VALUES ('run','conversation',?)", (json.dumps(seal),))
    connection.execute("INSERT INTO run_results VALUES ('run',?)", (json.dumps(result),))
    envelope = {"session_id": "conversation", "turn_id": "run", "revision": 3, "cursor": {"sequence": 10}}
    permission = {"request_id": "permission", "permission_domain": "edit", "rationale": "overwrite existing fixture",
                  "decision": None}
    events = []

    def event(sequence, kind, value):
        events.append((sequence, kind, {"type": kind, **value}))

    def part(sequence, call, capability, state, update):
        value = {"kind": "tool", "id": "tool:" + call, "call_id": call, "capability_id": capability,
            "turn_id": "run", "message_id": "assistant", "state": state, "input": {"path": str(target)},
            "permission": {"input_preview": "bounded permission", "truncated": True} if call == "write" else None}
        event(sequence, "part_updated", {"diagnostic_schema": "nola.part_event.v1", "turn_id": "run",
            "projection_revision": sequence, "part": value, "update": {"part_id": "tool:" + call, **update}})

    part(1, "write", "write_file", "waiting_for_permission", {"type": "tool_permission", "permission": permission})
    event(2, "permission_settled", {"receipt": {"request_id": "permission", "turn_id": "run", "call_id": "write",
        "capability_id": "write_file", "resource": resource, "outcome": {"type": "authorization", "decision": decision}},
        "core_envelope": envelope})
    part(3, "write", "write_file", "pending" if decision == "allow_once" else "cancelled",
         {"type": "tool_permission", "permission": {**permission, "decision": decision}})
    connection.execute("INSERT INTO host_event_inbox VALUES ('run','permission-effect','permission-rpc','permission-event',?)",
        (json.dumps({"session_id": "conversation", "turn_id": "run", "effect_id": "permission-effect",
            "request_id": "permission-rpc", "payload": {"type": "permission_resolved", "request_id": "permission",
                "resolution": {"type": "authorization", "decision": decision}}}),))

    def executed(call, capability, start_sequence, output):
        effect, request, attempt = call + "-effect", call + "-request", call + "-attempt"
        delivery = {"attempt_id": attempt, "worker_generation": "worker", "lease_owner": "owner", "lease_token": "token"}
        response = {"session_id": "conversation", "turn_id": "run", "host_id": "fixture-host",
            "effect_id": effect, "request_id": request, "sequence": 2, "catalog_revision": 4, "availability_epoch": 1,
            "delivery": delivery, "payload": {"type": "tool_completed", "call_id": call, "outcome": "succeeded", "output": output}}
        record = {"id": attempt, "run_id": "run", "effect_id": effect, "request_id": request,
            "state": "settled", "settlement": "succeeded", "execution_started_at_ms": start_sequence,
            **{key: delivery[key] for key in ("worker_generation", "lease_owner", "lease_token")}}
        connection.execute("INSERT INTO host_event_inbox VALUES ('run',?,?,?,?)",
                           (effect, request, call + "-event", json.dumps(response)))
        connection.execute("INSERT INTO effect_delivery_attempts VALUES (?,'run',?,?,'settled',?)",
                           (attempt, effect, request, json.dumps(record)))
        connection.execute("INSERT INTO execution_evidence_sources VALUES ('run',?,'conversation','run',?,'conversation',?,3,10,?)",
                           (call + "-evidence", call, call + "-event", f"event-{start_sequence + 1}"))
        part(start_sequence, call, capability, "running", {"type": "tool_state", "state": "running",
             "timestamp_ms": start_sequence, "snapshot_in": "part"})
        event(start_sequence + 1, "execution_evidence_observed", {"turn_id": "run", "core_envelope": envelope,
            "evidence": {"evidence_id": call + "-evidence", "call_id": call, "tool_name": capability,
                "outcome": "succeeded", "receipt": output["execution"], "source": {"attempt_id": None,
                    "effect_id": effect, "request_id": request, "host_id": "fixture-host", "response_sequence": "2",
                    "catalog_revision": "4", "availability_epoch": "1"}}})
        part(start_sequence + 2, call, capability, "completed", {"type": "tool_state", "state": "completed",
             "timestamp_ms": start_sequence + 2, "snapshot_in": "part"})

    if decision == "allow_once":
        executed("write", "write_file", 4, {"contract": "nola.tool_result.v2", "execution": {"state": "completed", "local_stop": None,
            "checks": [], "effects": [{"operation": "write", "resource_uri": target.as_uri(),
                "before_version": "sha256:" + digest(original), "after_version": "sha256:" + digest(expected)}]},
            "presentation": {"summary": "Wrote fixture", "text": "Wrote fixture"}})
        executed("read", "read_file", 7, {"contract": "nola.tool_result.v2", "execution": None,
            "presentation": {"summary": "Read fixture", "text": "# " + str(target) + "\nlines: 1-1 of 1\n\n   1 | approved"}})
    event(20, "run_result_decided", {"decision": {"result_id": "result", "value": result["value"]}})
    event(21, "turn_completed", {"turn_id": "run"})
    connection.executemany("INSERT INTO run_events VALUES ('run',?,?,?,?)",
                           [(seq, kind, json.dumps(value), f"event-{seq}") for seq, kind, value in events])
    return connection, presentation, original, expected


@pytest.mark.parametrize("decision", ["allow_once", "deny"])
def test_exact_approval_or_denial_has_independent_cold_evidence(tmp_path, decision):
    connection, presentation, original, expected = fixture(tmp_path, decision)
    result = auditor.audit(connection, presentation, tmp_path, decision, original, expected)
    assert result["passed"] is True
    assert result["physical_executions"] == int(decision == "allow_once")


def mutate_json(connection, table, column, where, mutate):
    rowid, raw = connection.execute(f"SELECT rowid,{column} FROM {table} WHERE {where}").fetchone()
    value = json.loads(raw)
    mutate(value)
    connection.execute(f"UPDATE {table} SET {column}=? WHERE rowid=?", (json.dumps(value), rowid))


@pytest.mark.parametrize("call", ["write", "read"])
def test_only_tool_result_v2_is_admitted_without_a_v1_fallback(tmp_path, call):
    connection, presentation, original, expected = fixture(tmp_path, "allow_once")
    mutate_json(connection, "host_event_inbox", "payload_json", f"event_id='{call}-event'",
        lambda value: value["payload"]["output"].update(contract="nola.tool_result.v1"))
    with pytest.raises(ValueError, match="ToolResult V2"):
        auditor.audit(connection, presentation, tmp_path, "allow_once", original, expected)


def test_v2_requires_explicit_execution_even_when_no_receipt_is_available(tmp_path):
    connection, presentation, original, expected = fixture(tmp_path, "allow_once")
    mutate_json(connection, "host_event_inbox", "payload_json", "event_id='read-event'",
        lambda value: value["payload"]["output"].pop("execution"))
    with pytest.raises(ValueError, match="explicit execution"):
        auditor.audit(connection, presentation, tmp_path, "allow_once", original, expected)


def test_v2_receipt_missing_local_stop_cannot_be_repaired_by_matching_canonical_evidence(tmp_path):
    connection, presentation, original, expected = fixture(tmp_path, "allow_once")
    mutate_json(connection, "host_event_inbox", "payload_json", "event_id='write-event'",
        lambda value: value["payload"]["output"]["execution"].pop("local_stop"))
    mutate_json(connection, "run_events", "payload_json", "event_kind='execution_evidence_observed' AND sequence=5",
        lambda value: value["evidence"]["receipt"].pop("local_stop"))
    with pytest.raises(ValueError, match="explicit local_stop null"):
        auditor.audit(connection, presentation, tmp_path, "allow_once", original, expected)


def test_file_auditor_does_not_accept_a_self_asserted_local_process_stop(tmp_path):
    connection, presentation, original, expected = fixture(tmp_path, "allow_once")
    stop = {"identity": {"execution_id":"self-asserted", "host_id":"fixture-host", "session_id":"conversation",
        "turn_id":"run", "effect_id":"write-effect", "request_id":"write-request", "call_id":"write",
        "delivery_attempt_id":"write-attempt"}, "scope":{"kind":"owned_process_group", "leader_pid":123}}
    mutate_json(connection, "host_event_inbox", "payload_json", "event_id='write-event'",
        lambda value: value["payload"]["output"]["execution"].update(local_stop=stop))
    mutate_json(connection, "run_events", "payload_json", "event_kind='execution_evidence_observed' AND sequence=5",
        lambda value: value["evidence"]["receipt"].update(local_stop=stop))
    with pytest.raises(ValueError, match="no local stop is proven"):
        auditor.audit(connection, presentation, tmp_path, "allow_once", original, expected)


def test_permission_verified_boolean_cannot_replace_formal_settlement(tmp_path):
    connection, presentation, original, expected = fixture(tmp_path, "deny")
    connection.execute("DELETE FROM run_events WHERE event_kind='permission_settled'")
    with pytest.raises(ValueError, match="permission lifecycle"):
        auditor.audit(connection, presentation, tmp_path, "deny", original, expected)


def test_foreign_host_permission_response_is_rejected(tmp_path):
    connection, presentation, original, expected = fixture(tmp_path, "deny")
    mutate_json(connection, "host_event_inbox", "payload_json", "event_id='permission-event'",
                lambda value: value.update(turn_id="foreign-run"))
    with pytest.raises(ValueError, match="Host authorization"):
        auditor.audit(connection, presentation, tmp_path, "deny", original, expected)


def test_denied_call_cannot_have_a_current_part_execution(tmp_path):
    connection, presentation, original, expected = fixture(tmp_path, "deny")
    raw = connection.execute("SELECT payload_json FROM run_events WHERE sequence=3").fetchone()[0]
    value = json.loads(raw)
    value["part"]["state"] = "running"
    value["update"] = {"type": "tool_state", "part_id": "tool:write", "state": "running", "snapshot_in": "part"}
    connection.execute("INSERT INTO run_events VALUES ('run',4,'part_updated',?,'event-4')", (json.dumps(value),))
    with pytest.raises(ValueError, match="denied action executed"):
        auditor.audit(connection, presentation, tmp_path, "deny", original, expected)


def test_second_physical_execution_is_not_hidden_by_one_completion(tmp_path):
    connection, presentation, original, expected = fixture(tmp_path, "allow_once")
    connection.execute("INSERT INTO effect_delivery_attempts SELECT 'replay',run_id,effect_id,request_id,state,record_json "
                       "FROM effect_delivery_attempts WHERE id='write-attempt'")
    with pytest.raises(ValueError, match="physical delivery count"):
        auditor.audit(connection, presentation, tmp_path, "allow_once", original, expected)


def test_uncompleted_unknown_delivery_cannot_be_hidden_on_denial(tmp_path):
    connection, presentation, original, expected = fixture(tmp_path, "deny")
    connection.execute("INSERT INTO effect_outbox VALUES ('run','unknown-effect')")
    connection.execute("INSERT INTO effect_outbox_payloads VALUES ('unknown-effect',?)",
                       (json.dumps({"payload": {"type": "execute_tool", "call_id": "write"}}),))
    connection.execute("INSERT INTO effect_delivery_attempts VALUES ('unknown','run','unknown-effect','rpc','outcome_unknown',?)",
                       (json.dumps({"state": "outcome_unknown", "execution_started_at_ms": 5}),))
    with pytest.raises(ValueError, match="physical delivery count"):
        auditor.audit(connection, presentation, tmp_path, "deny", original, expected)


def test_receipt_cannot_substitute_another_physical_attempt(tmp_path):
    connection, presentation, original, expected = fixture(tmp_path, "allow_once")
    mutate_json(connection, "host_event_inbox", "payload_json", "event_id='write-event'",
                lambda value: value["delivery"].update(attempt_id="other-attempt"))
    with pytest.raises(ValueError, match="physical delivery"):
        auditor.audit(connection, presentation, tmp_path, "allow_once", original, expected)


def test_execution_evidence_requires_real_host_binding(tmp_path):
    connection, presentation, original, expected = fixture(tmp_path, "allow_once")
    connection.execute("UPDATE execution_evidence_sources SET host_event_id='other-event' WHERE call_id='write'")
    with pytest.raises(ValueError, match="durable Host source"):
        auditor.audit(connection, presentation, tmp_path, "allow_once", original, expected)


def test_execution_evidence_requires_exact_committed_frontier(tmp_path):
    connection, presentation, original, expected = fixture(tmp_path, "allow_once")
    connection.execute("UPDATE execution_evidence_sources SET core_cursor=99 WHERE call_id='write'")
    with pytest.raises(ValueError, match="durable Host source"):
        auditor.audit(connection, presentation, tmp_path, "allow_once", original, expected)


def test_write_receipt_must_prove_original_and_final_versions(tmp_path):
    connection, presentation, original, expected = fixture(tmp_path, "allow_once")
    mutate_json(connection, "host_event_inbox", "payload_json", "event_id='write-event'",
        lambda value: value["payload"]["output"]["execution"]["effects"][0].update(before_version="sha256:wrong"))
    with pytest.raises(ValueError, match="execution receipt differs"):
        auditor.audit(connection, presentation, tmp_path, "allow_once", original, expected)


def test_readback_uses_host_output_not_bounded_part_or_final_answer(tmp_path):
    connection, presentation, original, expected = fixture(tmp_path, "allow_once")
    mutate_json(connection, "host_event_inbox", "payload_json", "event_id='read-event'",
                lambda value: value["payload"]["output"]["presentation"].update(text="different file content"))
    with pytest.raises(ValueError, match="Host read-back"):
        auditor.audit(connection, presentation, tmp_path, "allow_once", original, expected)


def test_marker_in_other_prose_is_not_complete_file_readback(tmp_path):
    connection, presentation, original, expected = fixture(tmp_path, "allow_once")
    mutate_json(connection, "host_event_inbox", "payload_json", "event_id='read-event'",
                lambda value: value["payload"]["output"]["presentation"].update(text="The word approved appeared elsewhere"))
    with pytest.raises(ValueError, match="Host read-back"):
        auditor.audit(connection, presentation, tmp_path, "allow_once", original, expected)


def test_real_readback_can_explicitly_disable_line_numbers(tmp_path):
    connection, presentation, original, expected = fixture(tmp_path, "allow_once")
    mutate_json(connection, "run_events", "payload_json", "sequence=7",
                lambda value: value["part"]["input"].update(show_line_numbers=False))
    mutate_json(connection, "host_event_inbox", "payload_json", "event_id='read-event'",
        lambda value: value["payload"]["output"]["presentation"].update(
            text=f"# {tmp_path / 'existing.txt'}\nlines: 1-1 of 1\n\n" + expected.decode()))
    assert auditor.audit(connection, presentation, tmp_path, "allow_once", original, expected)["passed"]


def test_mutated_predecision_bytes_or_foreign_owner_are_rejected(tmp_path):
    connection, presentation, original, expected = fixture(tmp_path, "deny")
    observation = presentation["permission_audits"][0]
    observation["fixture_bytes_base64"] = base64.b64encode(b"already changed").decode()
    with pytest.raises(ValueError, match="original bytes/hash"):
        auditor.audit(connection, presentation, tmp_path, "deny", original, expected)
    observation["fixture_bytes_base64"] = base64.b64encode(original).decode()
    observation["run_id"] = "other-run"
    with pytest.raises(ValueError, match="another owner"):
        auditor.audit(connection, presentation, tmp_path, "deny", original, expected)
