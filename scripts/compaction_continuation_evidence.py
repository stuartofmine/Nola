#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Read committed compaction evidence, never a client trace projection."""
import json
import sqlite3
from pathlib import Path


def verify(database_path):
    uri = Path(database_path).resolve().as_uri() + "?mode=ro"
    with sqlite3.connect(uri, uri=True) as db:
        rows = db.execute(
            "SELECT run_id, sequence, event_kind, payload_json FROM run_events "
            "WHERE event_kind IN ('host_progress', 'compaction_requested', 'compaction_completed') "
            "ORDER BY run_id, sequence"
        ).fetchall()
    attempts = {}
    accepted = {}
    plans = {}
    for run_id, sequence, kind, payload in rows:
        event = json.loads(payload)
        if kind == "compaction_requested":
            plans[run_id] = (sequence, event.get("source_fingerprint"))
            accepted.pop(run_id, None)
        elif kind == "host_progress" and event.get("title") == "compaction_request_metrics":
            metadata = event.get("metadata", {})
            effect_id = event.get("effect_id")
            turn_id = event.get("turn_id")
            if not effect_id or not turn_id:
                continue
            key = (run_id, turn_id, effect_id)
            if metadata.get("accepted_summary") is False and metadata.get("continuation_count") == 0:
                attempts[key] = (sequence, metadata.get("request_digest"))
            prior = attempts.get(key)
            plan = plans.get(run_id)
            if (prior and plan and plan[0] < prior[0] < sequence
                    and metadata.get("strategy") == "summary_continuation"
                    and metadata.get("accepted_summary") is True
                    and metadata.get("continuation_count", 0) > 0
                    and metadata.get("partial_summary_bytes", 0) > 0
                    and prior[1] == metadata.get("request_digest")):
                accepted[run_id] = {
                    "sequence": sequence,
                    "continuation_count": metadata["continuation_count"],
                    "partial_summary_bytes": metadata["partial_summary_bytes"],
                }
        elif kind == "compaction_completed":
            evidence = accepted.get(run_id)
            plan = plans.pop(run_id, None)
            if (evidence and plan and evidence["sequence"] < sequence
                    and event.get("source_fingerprint")
                    and event["source_fingerprint"] == plan[1]
                    and event.get("compaction_item_id")
                    and isinstance(event.get("summary"), str)
                    and event["summary"].strip()):
                return evidence
    raise ValueError("No committed compaction follows a correlated partial-summary continuation")


if __name__ == "__main__":
    import sys
    print(json.dumps(verify(sys.argv[1]), sort_keys=True))
