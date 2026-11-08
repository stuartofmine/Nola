"""Exercise the actual runner receipt block without launching another Mac app."""

import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
import real_e2e_evidence


class MacXCTestReceiptTests(unittest.TestCase):
    def run_receipt(self, summary, model_requests=2):
        source = (SCRIPTS / "run_real_mac_xctests.sh").read_text()
        block = source.rsplit("<<'PY'\n", 1)[1].split("\nPY\n", 1)[0]
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            result = base / "result.xcresult"
            result.with_suffix(".summary.json").write_text(json.dumps(summary))
            provider = base / "provider.ndjson"
            rows = [{"event": "provider.response", "status": 200,
                     "responseBytes": 100, "upstreamPath": "/v1/models"}]
            rows += [{"event": "provider.response", "status": 200,
                      "responseBytes": 100, "upstreamPath": "/v1/chat/completions",
                      "model": "fixture"} for _ in range(model_requests)]
            provider.write_text("\n".join(map(json.dumps, rows)))
            args = ["receipt", str(SCRIPTS.parent), str(result), str(base / "log"), "1"]
            args += [str(base / name) for name in
                     ("db", "screenshot", "accessibility", "timeline", "transcript", "events")]
            args += [str(provider), "fixture"]
            with patch.object(sys, "argv", args), patch.dict(os.environ, {
                "NOLA_MAC_REAL_REQUIRE_TEXT_STREAM": "0"
            }), patch.object(real_e2e_evidence, "record_step") as record:
                exec(compile(block, "runner-receipt", "exec"), {})
                return record.call_args.kwargs

    def test_counts_provider_requests_not_tests_or_discovery(self):
        result = self.run_receipt({"totalTestCount": 1, "passedTests": 1,
                                   "failedTests": 0, "skippedTests": 0})
        self.assertEqual(result["real_provider_requests"], 2)
        self.assertEqual(result["metrics"]["selectedTestCount"], 1)
        for name in ("selected_real_tests_executed", "selected_real_tests_passed",
                     "no_selected_test_skipped"):
            self.assertEqual(result["assertion_evidence"][name], ["mac-xctest_summary"])

    def test_discovery_only_does_not_invent_model_request(self):
        result = self.run_receipt({"totalTestCount": 1, "passedTests": 1,
                                   "failedTests": 0, "skippedTests": 0}, 0)
        self.assertEqual(result["real_provider_requests"], 0)

    def test_missing_failed_skipped_or_wrong_count_rejected(self):
        good = {"totalTestCount": 1, "passedTests": 1, "failedTests": 0, "skippedTests": 0}
        for summary in ({}, {**good, "failedTests": 1}, {**good, "skippedTests": 1},
                        {**good, "totalTestCount": 2}, {**good, "passedTests": 0}):
            with self.subTest(summary=summary), self.assertRaises(SystemExit):
                self.run_receipt(summary)


if __name__ == "__main__":
    unittest.main()
