# SPDX-License-Identifier: Apache-2.0
"""Cross-check committed Run evidence, not diagnostic Trace lifecycle.

This checks relational consistency; it does not replace the production Rust
validator's cryptographic seal/manifest verification.
"""
import json


def audit_runs(connection, conversation_id):
    rows = connection.execute("""SELECT r.id,r.status,r.assistant_message_id,
        r.completed_at_ms,s.seal_json,rr.result_json
        FROM runs r LEFT JOIN assistant_run_terminal_seals s
        ON s.run_id=r.id AND s.conversation_id=r.conversation_id
        LEFT JOIN run_results rr ON rr.run_id=r.id
        WHERE r.conversation_id=? AND r.kind='conversation' ORDER BY r.created_at_ms,r.id""",
        (conversation_id,)).fetchall()
    if not rows:
        raise ValueError("conversation has no canonical Run")
    outcomes = []
    for run, status, assistant, completed, seal_raw, result_raw in rows:
        if status != "sealed" or completed is None or not seal_raw or not result_raw:
            raise ValueError("Run lacks a committed result and seal")
        seal, result = json.loads(seal_raw), json.loads(result_raw)
        identity = seal.get("identity", {})
        if (identity.get("run_id") != run or identity.get("conversation_id") != conversation_id
                or identity.get("assistant_message_id") != assistant
                or seal.get("completed_at_ms") != completed or seal.get("result") != result
                or result.get("run_id") != run or result.get("conversation_id") != conversation_id):
            raise ValueError("Run/result/seal identity or content mismatch")
        kind = result.get("value", {}).get("kind")
        if kind not in ("answered", "action_completed", "delegation_accepted", "incomplete", "blocked", "failed", "cancelled"):
            raise ValueError("unknown canonical outcome")
        if kind == "delegation_accepted":
            value = result["value"]
            authority = connection.execute("""SELECT phase,admission_json
                FROM runtime_agent_task_authorities WHERE task_id=?""", (value.get("task_id"),)).fetchone()
            if not authority or authority[0] != "admitted" or not authority[1]:
                raise ValueError("Delegation receipt lacks durable task admission")
            admission = json.loads(authority[1])
            if (admission.get("task_id") != value.get("task_id")
                    or admission.get("parent_run_id") != run
                    or admission.get("parent_conversation_id") != conversation_id
                    or admission.get("parent_tool_call_id") != value.get("parent_call_id")):
                raise ValueError("Delegation receipt does not match its exact parent admission")
        events = connection.execute("""SELECT event_kind,payload_json FROM run_events
            WHERE run_id=? AND event_kind IN
            ('run_result_decided','turn_completed','turn_failed','turn_cancelled')
            ORDER BY sequence""", (run,)).fetchall()
        decisions = [json.loads(raw).get("decision", {}) for event, raw in events
                     if event == "run_result_decided"]
        terminals = [event for event, _ in events if event != "run_result_decided"]
        expected_terminal = {"failed": "turn_failed", "cancelled": "turn_cancelled"}.get(
            kind, "turn_completed")
        # Failure/cancellation reducers may emit their terminal before deriving
        # the canonical decision in finish_transition. Normal completion must
        # still decide its result before publishing completion.
        if (len(decisions) != 1 or len(terminals) != 1
                or terminals[0] != expected_terminal
                or (expected_terminal == "turn_completed"
                    and events[0][0] != "run_result_decided")
                or decisions[0].get("result_id") != result.get("result_id")
                or decisions[0].get("value") != result.get("value")):
            raise ValueError("canonical decision or terminal is missing, duplicated, or inconsistent")
        outcomes.append(kind)
    return {"runCount": len(rows), "outcomes": outcomes}
