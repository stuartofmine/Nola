#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Compare production-validated transcript results with one SQLite snapshot.

This read-only gate does not implement the Rust seal/manifest validator. The
platform harness must first obtain canonical_run_terminal from Application.
The Mac success gate and central lifecycle gate share this comparison; lifecycle
sealing alone never implies the result is a successful answer or action.
"""
import json
import re


def verify(connection, transcript, minimum_runs, *, require_success=True):
    if not isinstance(transcript, list) or minimum_runs < 1:
        raise ValueError("invalid transcript acceptance input")
    if any(not isinstance(row, dict) for row in transcript):
        raise ValueError("transcript contains a non-object row")
    terminals = [row.get("canonicalTerminal") for row in transcript if row.get("role") == "assistant"]
    if len(terminals) < minimum_runs:
        raise ValueError("missing production-validated assistant terminals")
    seen, conversations, outcomes = set(), set(), {}
    connection.execute("BEGIN")
    try:
        for terminal in terminals:
            if not isinstance(terminal, dict):
                raise ValueError("assistant lacks production terminal evidence")
            identity = tuple(terminal.get(key) for key in
                             ("conversation_id", "run_id", "assistant_message_id"))
            if any(not isinstance(value, str) or not value for value in identity) or identity in seen:
                raise ValueError("missing or duplicate terminal identity")
            conversation, run, assistant = identity
            seen.add(identity)
            conversations.add(conversation)
            rows = connection.execute("""SELECT r.status,r.assistant_message_id,r.completed_at_ms,s.seal_json
                FROM runs r JOIN assistant_run_terminal_seals s
                ON s.run_id=r.id AND s.conversation_id=r.conversation_id
                WHERE r.id=? AND r.conversation_id=?""", (run, conversation)).fetchall()
            if len(rows) != 1:
                raise ValueError("exact Run has no unique terminal seal")
            status, owner, completed, raw = rows[0]
            seal = json.loads(raw)
            if (status != "sealed" or terminal.get("status") != "sealed" or owner != assistant
                    or completed != terminal.get("completed_at_ms")
                    or completed != seal.get("completed_at_ms")
                    or any(seal.get("identity", {}).get(key) != value for key, value in
                           zip(("conversation_id", "run_id", "assistant_message_id"), identity))
                    or not re.fullmatch(r"[0-9a-f]{64}", terminal.get("result_digest", ""))
                    or seal.get("result_digest") != terminal["result_digest"]
                    or seal.get("result") != terminal.get("result")):
                raise ValueError("reported terminal differs from persisted seal")
            result = seal.get("result")
            value = result.get("value") if isinstance(result, dict) else None
            kind = value.get("kind") if isinstance(value, dict) else None
            if kind not in ("answered", "action_completed", "incomplete", "blocked", "failed", "cancelled"):
                raise ValueError("unknown sealed result kind")
            outcomes[kind] = outcomes.get(kind, 0) + 1
            if require_success and kind not in ("answered", "action_completed"):
                raise ValueError("sealed Run is not a successful answer or action")
        expected = {(conversation, run) for conversation, run, _ in seen}
        actual = set()
        for conversation in conversations:
            actual.update((conversation, row[0]) for row in connection.execute(
                "SELECT id FROM runs WHERE conversation_id=? AND kind='conversation'", (conversation,)))
        if actual != expected:
            raise ValueError("conversation contains unaccounted Runs")
        return {"canonicalSealedRuns": len(seen), "outcomes": outcomes,
                "canonicalSealedSuccessfulRuns": sum(outcomes.get(kind, 0) for kind in ("answered", "action_completed")),
                "canonicalFailedRuns": outcomes.get("failed", 0),
                "authority": "production_result_compared_to_persisted_seal"}
    finally:
        connection.rollback()
