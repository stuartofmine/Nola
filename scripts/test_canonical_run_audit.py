# SPDX-License-Identifier: Apache-2.0
import json
import sqlite3

import pytest

from canonical_run_audit import audit_runs


def fixture(kind="action_completed", terminal_first=False):
    db = sqlite3.connect(":memory:")
    db.executescript("""
        CREATE TABLE runs(id,status,assistant_message_id,completed_at_ms,conversation_id,kind,created_at_ms);
        CREATE TABLE assistant_run_terminal_seals(run_id,conversation_id,seal_json);
        CREATE TABLE run_results(run_id,result_json);
        CREATE TABLE run_events(run_id,event_kind,payload_json,sequence);
        INSERT INTO runs VALUES('r','sealed','a',10,'c','conversation',1);
    """)
    result = {"result_id": "decision", "run_id": "r", "conversation_id": "c",
              "value": {"kind": kind}}
    seal = {"identity": {"run_id": "r", "conversation_id": "c", "assistant_message_id": "a"},
            "completed_at_ms": 10, "result": result}
    db.execute("INSERT INTO run_results VALUES('r',?)", (json.dumps(result),))
    db.execute("INSERT INTO assistant_run_terminal_seals VALUES('r','c',?)", (json.dumps(seal),))
    db.execute("INSERT INTO run_events VALUES('r','run_result_decided',?,1)",
               (json.dumps({"decision": result}),))
    terminal = {"failed": "turn_failed", "cancelled": "turn_cancelled"}.get(kind, "turn_completed")
    db.execute("INSERT INTO run_events VALUES('r',?,'{}',?)",
               (terminal, 0 if terminal_first else 2))
    return db


def test_canonical_result_without_trace():
    assert audit_runs(fixture(), "c")["outcomes"] == ["action_completed"]


@pytest.mark.parametrize("sql", [
    "DELETE FROM assistant_run_terminal_seals",
    "UPDATE runs SET assistant_message_id='foreign'",
    "INSERT INTO run_events VALUES('r','turn_completed','{}',3)",
    "UPDATE run_results SET result_json='{}'",
    "UPDATE run_events SET sequence=0 WHERE event_kind='turn_completed'",
])
def test_missing_or_conflicting_evidence_fails(sql):
    db = fixture()
    db.execute(sql)
    with pytest.raises(ValueError):
        audit_runs(db, "c")


def test_foreign_conversation_cannot_borrow_terminal():
    with pytest.raises(ValueError):
        audit_runs(fixture(), "foreign")


@pytest.mark.parametrize("kind", ["failed", "cancelled"])
@pytest.mark.parametrize("terminal_first", [False, True])
def test_runtime_terminal_and_decision_have_consistent_outcome_in_either_order(kind, terminal_first):
    assert audit_runs(fixture(kind, terminal_first), "c")["outcomes"] == [kind]


@pytest.mark.parametrize("kind", ["answered", "action_completed", "incomplete", "blocked"])
def test_normal_completion_requires_prior_decision(kind):
    assert audit_runs(fixture(kind), "c")["outcomes"] == [kind]
    with pytest.raises(ValueError):
        audit_runs(fixture(kind, terminal_first=True), "c")


@pytest.mark.parametrize("kind", ["failed", "cancelled", "action_completed"])
@pytest.mark.parametrize("mutation", [
    "DELETE FROM run_events WHERE event_kind='run_result_decided'",
    "DELETE FROM run_events WHERE event_kind!='run_result_decided'",
    "INSERT INTO run_events SELECT run_id,event_kind,payload_json,9 FROM run_events WHERE event_kind='run_result_decided'",
    "INSERT INTO run_events SELECT run_id,event_kind,payload_json,9 FROM run_events WHERE event_kind!='run_result_decided'",
    "UPDATE run_events SET payload_json='{}' WHERE event_kind='run_result_decided'",
    "UPDATE runs SET assistant_message_id='foreign'",
    "UPDATE run_results SET result_json='{}'",
    "DELETE FROM assistant_run_terminal_seals",
])
def test_runtime_outcomes_still_reject_inconsistent_evidence(kind, mutation):
    db = fixture(kind, terminal_first=kind != "action_completed")
    db.execute(mutation)
    with pytest.raises(ValueError):
        audit_runs(db, "c")


@pytest.mark.parametrize("kind,wrong_terminal", [
    ("failed", "turn_completed"), ("failed", "turn_cancelled"),
    ("cancelled", "turn_completed"), ("cancelled", "turn_failed"),
    ("action_completed", "turn_failed"), ("action_completed", "turn_cancelled"),
])
def test_terminal_type_must_match_canonical_outcome(kind, wrong_terminal):
    db = fixture(kind)
    db.execute("UPDATE run_events SET event_kind=? WHERE event_kind!='run_result_decided'",
               (wrong_terminal,))
    with pytest.raises(ValueError):
        audit_runs(db, "c")
