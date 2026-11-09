# SPDX-License-Identifier: Apache-2.0
"""Independent-oracle and fixture guards, not real-provider acceptance."""
import json
from pathlib import Path
import tempfile
import unittest

from test_real_cli_hundreds_steps import EventObserver, check_files, make_chain, provider_window_audit, verified_provider_recoveries


class HundredsStepOracleTests(unittest.TestCase):
    def fixture(self, root, count=320):
        project = root / "project"
        project.mkdir()
        expected = make_chain(project, count)
        records = [{"record": index, "value": value} for index, value in enumerate(expected["values"])]
        (project / "report.json").write_text(json.dumps({"records": records, "sum": sum(expected["values"])}))
        for batch, start in enumerate(range(0, count, 20)):
            selected = records[start:start + 20]
            (project / "checkpoints" / f"{batch:03d}.json").write_text(
                json.dumps({"records": selected, "sum": sum(row["value"] for row in selected)}))
        return project, expected

    def test_opaque_chain_has_one_actual_missing_link_and_recovery(self):
        with tempfile.TemporaryDirectory() as directory:
            project, expected = self.fixture(Path(directory))
            self.assertEqual(len(set(expected["names"])), 320)
            self.assertFalse((project / expected["missing"]).exists())
            for index, name in enumerate(expected["names"]):
                fields = dict(line.split("=", 1) for line in (project / name).read_text().splitlines())
                following = expected["names"][index + 1] if index < 319 else "END"
                if index == expected["break_after"]:
                    self.assertEqual(fields["next"], expected["missing"])
                    self.assertEqual(json.loads((project / fields["on_missing_next"]).read_text()), {"next": following})
                else:
                    self.assertEqual(fields["next"], following)
            self.assertEqual(check_files(project, expected)["records"], 320)

    def test_report_with_correct_count_but_wrong_value_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            project, expected = self.fixture(Path(directory))
            path = project / "report.json"
            report = json.loads(path.read_text())
            report["records"][157]["value"] += 1
            report["sum"] += 1
            path.write_text(json.dumps(report))
            with self.assertRaisesRegex(AssertionError, "report oracle"):
                check_files(project, expected)

    def test_modified_input_is_rejected_even_with_a_correct_report(self):
        with tempfile.TemporaryDirectory() as directory:
            project, expected = self.fixture(Path(directory))
            (project / expected["names"][0]).write_text("changed input")
            with self.assertRaisesRegex(AssertionError, "input evidence"):
                check_files(project, expected)

    def test_provider_usage_must_fit_the_window_with_requested_output_reserved(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "provider.ndjson"
            row = {"status": 200, "requestedOutputTokens": 8192, "responseUsage": {"input_tokens": 55122}}
            path.write_text(json.dumps(row))
            audit = provider_window_audit(path)
            self.assertEqual(audit["remaining_window_at_peak"], 2222)
            self.assertEqual(audit["requests_exceeding_estimated_input_limit"], 1)
            row["responseUsage"]["input_tokens"] = 58000
            path.write_text(json.dumps(row))
            with self.assertRaisesRegex(AssertionError, "exceeds the configured model window"):
                provider_window_audit(path)
            del row["responseUsage"]
            path.write_text(json.dumps(row))
            with self.assertRaisesRegex(AssertionError, "evidence is incomplete"):
                provider_window_audit(path)

    def test_transport_failure_requires_an_observed_successful_retry_of_identical_input(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "provider.ndjson"
            failed = {"forwarded": True, "status": 502, "requestSha256": "original", "error": "TLS EOF"}
            different = {"forwarded": True, "status": 200, "requestSha256": "different", "upstreamRequestSha256": "different"}
            path.write_text("\n".join(map(json.dumps, [failed, different])))
            with self.assertRaisesRegex(AssertionError, "no observed successful retry"):
                verified_provider_recoveries(path)
            same = {**different, "requestSha256": "original", "upstreamRequestSha256": "original", "requestId": "recovered"}
            path.write_text("\n".join(map(json.dumps, [failed, same])))
            self.assertEqual(verified_provider_recoveries(path)[0]["recovered_request_id"], "recovered")
            path.write_text(json.dumps({**same, "injectedInterruption": True}))
            with self.assertRaisesRegex(AssertionError, "response injection"):
                verified_provider_recoveries(path)
            path.write_text(json.dumps({**same, "upstreamRequestSha256": "changed"}))
            with self.assertRaisesRegex(AssertionError, "relay changed"):
                verified_provider_recoveries(path)

    def test_current_output_slice_tool_is_allowed_but_shell_is_still_forbidden(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            project, expected = self.fixture(root, 4)
            path = root / "events.ndjson"
            events = [{"trace":{"kind":"tool.completed","sequence":index,"status":"succeeded",
                "payload":{"call_id":str(index),"tool_name":name,"arguments":{}}}}
                for index, name in enumerate(["tool_output_read", "bash", "read_tool_output"], 1)]
            path.write_text("\n".join(map(json.dumps, events)) + "\n")
            observer = EventObserver(path, project, expected)
            try:
                observer.update()
                self.assertEqual(observer.other_tools, {"bash", "read_tool_output"})
            finally:
                observer.handle.close()

    def test_duplicate_stream_receipt_is_not_another_executed_node(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            project, expected = self.fixture(root, 4)
            event = {"trace": {"kind": "tool.completed", "sequence": 1, "status": "succeeded",
                "payload": {"call_id": "one-call", "tool_name": "read_file", "arguments": {"path": expected["names"][0]}}}}
            path = root / "events.ndjson"
            path.write_text(json.dumps(event) + "\n" + json.dumps(event) + "\n")
            observer = EventObserver(path, project, expected)
            try:
                observer.update()
                self.assertEqual(observer.snapshot()["unique_nodes_read"], 1)
                self.assertEqual(len(observer.reads), 1)
                self.assertTrue(observer.sequence_errors)
            finally:
                observer.handle.close()


if __name__ == "__main__":
    unittest.main()
