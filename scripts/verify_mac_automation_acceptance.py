#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Read-only acceptance audit; production Application owns seal validation.

Compare the exact occurrence receipt emitted by the Mac harness against its
persisted binding and immutable seal in one SQLite read transaction. This is
not an alternate executor or a replacement for Application's seal validator.
"""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import sqlite3


def verify(report: dict, expected_source: str) -> dict:
    chain = report.get("automation_chain", {})
    required = ("matched", "source_refreshed", "source_consumed", "skill_loaded",
                "output_written", "activity_recorded", "autonomous_orchestration_complete")
    if (not expected_source or chain.get("skipped") is True
            or any(chain.get(key) is not True for key in required)):
        raise ValueError("automation scenario did not satisfy canonical acceptance gates")
    identifiers = ("occurrence_id", "occurrence_key", "automation_id", "conversation_id",
                   "run_id", "assistant_message_id", "seal_digest")
    if any(not isinstance(chain.get(key), str) or not chain[key] for key in identifiers):
        raise ValueError("automation receipt is missing exact execution identities")
    if not re.fullmatch(r"[0-9a-f]{64}", chain["seal_digest"]):
        raise ValueError("invalid seal identity")
    root = Path(chain["fixture_root"]).resolve(strict=True)
    database = Path(chain["canonical_database"]).resolve(strict=True)
    artifact = Path(chain["output_path"]).resolve(strict=True)
    if not database.is_relative_to(root) or not artifact.is_relative_to(root):
        raise ValueError("automation evidence escaped the isolated fixture")
    with sqlite3.connect(database.as_uri() + "?mode=ro", uri=True) as connection:
        connection.execute("BEGIN")
        row = connection.execute("""
            SELECT occurrence.automation_id, occurrence.occurrence_key,
                   binding.binding_json, receipt.receipt_json, seal.seal_json, run.status
            FROM automation_runs occurrence
            JOIN automation_execution_bindings binding ON binding.occurrence_id = occurrence.id
            JOIN automation_execution_receipts receipt ON receipt.occurrence_id = occurrence.id
            JOIN runs run ON run.id = binding.run_id AND run.conversation_id = binding.conversation_id
            JOIN assistant_run_terminal_seals seal ON seal.run_id = run.id
                AND seal.conversation_id = run.conversation_id
            WHERE occurrence.id = ?
        """, (chain["occurrence_id"],)).fetchall()
        if len(row) != 1:
            raise ValueError("exact occurrence has no unique persisted terminal receipt")
        automation_id, occurrence_key, binding_json, receipt_json, seal_json, state = row[0]
        binding, receipt, seal = map(json.loads, (binding_json, receipt_json, seal_json))
        if (automation_id != chain["automation_id"] or occurrence_key != chain["occurrence_key"]
                or state != "sealed" or receipt != chain.get("canonical_receipt")
                or receipt.get("binding") != binding or receipt.get("terminal") != seal):
            raise ValueError("reported receipt differs from the canonical occurrence or seal")
        identity = seal.get("identity", {})
        if (binding.get("occurrence_id") != chain["occurrence_id"]
                or binding.get("conversation_id") != chain["conversation_id"]
                or binding.get("run_id") != chain["run_id"]
                or identity.get("conversation_id") != chain["conversation_id"]
                or identity.get("run_id") != chain["run_id"]
                or identity.get("assistant_message_id") != chain["assistant_message_id"]
                or seal.get("seal_digest") != chain["seal_digest"]
                or seal.get("result", {}).get("value", {}).get("kind") != "action_completed"):
            raise ValueError("canonical automation execution identity or outcome does not match")
    output = json.loads(artifact.read_text(encoding="utf-8"))
    if (not isinstance(output, dict) or output.get("status") != "validated"
            or output.get("skill") != "analyze-data" or output.get("source") != expected_source):
        raise ValueError("automation artifact failed independent readback")
    return {"passed": True, **{key: chain[key] for key in identifiers},
            "canonical_database": str(database), "output_path": str(artifact),
            "authority": "production_application_receipt_compared_to_persisted_seal"}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("result", type=Path)
    parser.add_argument("--receipt", required=True, type=Path)
    arguments = parser.parse_args()
    try:
        result = verify(json.loads(arguments.result.read_text(encoding="utf-8")),
                        os.environ.get("NOLA_MAC_RUNTIME_ACCEPTANCE_AUTOMATION_SOURCE_TOKEN", ""))
    except (OSError, ValueError, TypeError, KeyError, AttributeError, sqlite3.Error):
        # Do not print model output, raw database JSON or credentials on failure.
        result = {"passed": False, "reason": "automation_canonical_evidence_mismatch"}
    arguments.receipt.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    return 0 if result["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
