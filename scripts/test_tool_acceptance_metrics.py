import json
from pathlib import Path
import tempfile
import unittest
import sqlite3

from summarize_tool_acceptance import distribution, summarize


class ToolAcceptanceMetricsTests(unittest.TestCase):
    def test_missing_arguments_do_not_prove_same_argument_retries(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            turn = root / "turn-1"
            turn.mkdir()
            rows = [{"trace": {"kind": "tool.completed", "status": "succeeded",
                     "payload": {"call_id": str(index), "tool_name": "fixture"}}}
                    for index in range(2)]
            events = turn / "events.ndjson"
            events.write_text("\n".join(map(json.dumps, rows)))
            result = summarize(root)
            self.assertIsNone(result["sameArgumentRepeatCount"])
            self.assertEqual(result["sameArgumentComparableCalls"], 0)
            for row in rows:
                row["trace"]["payload"]["arguments"] = {}
            events.write_text("\n".join(map(json.dumps, rows)))
            result = summarize(root)
            self.assertEqual(result["sameArgumentRepeatCount"], 1)
            self.assertEqual(result["sameArgumentComparableCalls"], 2)

    def test_no_progress_rejections_are_deduplicated_and_require_structured_skip(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            turn = root / "turn-1"
            turn.mkdir()
            def row(call_id, detail):
                return {"trace":{"kind":"tool.completed", "status":"failed", "payload":{
                    "call_id":call_id, "tool_name":"fixture", "arguments":{}, "detail":detail}}}
            rejection = row("one", json.dumps({"error":"repeated_tool_batch", "tool_execution_skipped":True}))
            (turn / "events.ndjson").write_text("\n".join(json.dumps(value) for value in [
                rejection, rejection, row("two", "repeated_tool_batch"),
                row("three", json.dumps({"error":"repeated_tool_batch"}))]))
            result = summarize(root)
            self.assertEqual(result["reportedNoProgressRejections"], {"count":1,"structuredSkippedResults":1})
            self.assertIsNone(result["invalidRetryCount"])

    def test_host_discovery_metrics_and_model_probe_exclusion(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            turn = root / "turn-1"
            turn.mkdir()
            (turn / "provider.ndjson").write_text("\n".join(json.dumps(row) for row in [
                {"requestInputPresent":False,"durationMilliseconds":100,"tools":[]},
                {"requestInputPresent":True,"durationMilliseconds":200,"tools":["one"],
                 "toolSchemaUtf8Bytes":24,"toolSchemaEstimatedTokens":6,"toolSchemaTokenMethod":"ascii4_nonascii1_json"}]))
            with sqlite3.connect(root / "fixture.db") as db:
                db.execute("CREATE TABLE host_event_inbox(payload_json TEXT)")
                event = {"turn_id":"turn", "payload":{"type":"tool_completed","call_id":"call",
                    "output":{"data":{"tool_discovery_metrics":{"duration_us":12,
                        "returned_schema_utf8_bytes":20,"output_utf8_bytes":30}}}}}
                db.executemany("INSERT INTO host_event_inbox VALUES(?)", [(json.dumps(event),)] * 2)
                event["payload"]["call_id"] = "read"
                event["payload"]["output"]["data"] = {"type":"tool_output_slice","content":"中🙂","bytes_returned":7}
                db.executemany("INSERT INTO host_event_inbox VALUES(?)", [(json.dumps(event),)] * 2)
            result = summarize(root)
            self.assertEqual(result["providerRequestLatencyMs"], distribution([200]))
            self.assertEqual(result["discoveryLatencyUs"], distribution([12]))
            self.assertEqual(result["discoveredSchemaUtf8Bytes"], distribution([20]))
            self.assertEqual(result["discoveryOutputUtf8Bytes"], distribution([30]))
            self.assertEqual(result["requestToolSchemaUtf8Bytes"], distribution([24]))
            self.assertEqual(result["requestToolSchemaEstimatedTokens"]["p50"], 6)
            self.assertEqual(result["outputReadReturnedBytes"], {"total":7, **distribution([7])})

    def test_empty_distribution_is_unknown(self):
        self.assertEqual(distribution([]), {"samples": 0, "p50": None, "p95": None})
        self.assertEqual(distribution([3, 1, 2]), {"samples": 3, "p50": 2, "p95": 3})

    def test_replayed_event_is_not_a_retry_or_new_sample(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            turn = root / "turn-1"
            turn.mkdir()
            def row(call_id):
                return {"trace": {"kind": "tool.completed", "status": "failed",
                                  "payload": {"call_id": call_id, "tool_name": "fixture",
                                              "arguments": {"secret": "never emit me"}}}}
            (turn / "events.ndjson").write_text("\n".join(json.dumps(value) for value in
                                                         [row("a"), row("a"), row("b")]))
            result = summarize(root)
            self.assertEqual(result["toolOutcomes"], {"failed": 2})
            self.assertEqual(result["sameArgumentRepeatCount"], 1)
            self.assertEqual(result["toolFailureRateKnownOutcomes"], 1)
            self.assertIsNone(result["toolSchemaTokens"])
            self.assertNotIn("never emit me", json.dumps(result))


if __name__ == "__main__":
    unittest.main()
