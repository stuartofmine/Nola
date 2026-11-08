import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from provider_trace_metrics import provider_trace_metrics
from real_e2e_evidence import _observation
from real_e2e_judge import _judge_provider_real, _provider_metrics


class ProviderTraceMetricsTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.path = Path(self.directory.name) / "provider.ndjson"

    def write(self, rows):
        self.path.write_text("\n".join(map(json.dumps, rows)))
        return {"steps": [{"observations": [{"kind": "provider_trace", "path": str(self.path)}]}]}

    def row(self, route="/v1/chat/completions", **values):
        return {"event": "provider.response", "upstreamPath": route,
                "upstreamHost": "provider.example", "model": "fixture",
                "status": 200, "responseBytes": 42, **values}

    def test_protocols_discovery_failures_and_shared_ownership(self):
        receipt = self.write([
            self.row(), self.row("/v1/responses?test=1"), self.row("/v1/messages"),
            self.row("/v1/models"), self.row("/v1/responses/compact"),
            self.row(status=503), self.row(status=None, responseBytes=0),
        ])
        metrics = provider_trace_metrics([self.path, self.path])
        self.assertEqual(metrics["observedModelRequests"], 5)
        self.assertEqual(metrics["successfulModelRequests"], 3)
        self.assertEqual(metrics["completedModelStreams"], 0)
        self.assertEqual(metrics, _provider_metrics(receipt))
        self.assertEqual(metrics, _observation({"id": "probe", "kind": "provider_trace",
                                               "path": str(self.path)})["metrics"])

    def test_http_success_is_not_stream_completion(self):
        rows = [self.row("/v1/responses", stream=True, finishReasons=[reason],
                         responseOversizedLineCount=0) for reason in ("completed", "incomplete")]
        rows += [self.row(stream=True, finishReasons=["stop"], responseOversizedLineCount=0,
                          responseDoneMarkerCount=1, error="disconnected")]
        self.write(rows)
        metrics = provider_trace_metrics([self.path])
        self.assertEqual(metrics["successfulModelRequests"], 3)
        self.assertEqual(metrics["completedModelStreams"], 1)

    def test_unrelated_remote_discovery_does_not_prove_remote_inference(self):
        receipt = self.write([self.row(upstreamHost="localhost"), self.row("/v1/models")])
        self.assertFalse(_judge_provider_real(receipt)["passed"])

    def test_malformed_and_unreadable_evidence_fail_closed(self):
        receipt = self.write([self.row(), [], self.row(status="200"),
                              self.row(responseBytes=True)])
        self.assertEqual(_provider_metrics(receipt)["malformedRows"], 3)
        self.assertFalse(_judge_provider_real(receipt)["passed"])
        self.path.unlink()
        self.assertEqual(_provider_metrics(receipt)["unreadableFiles"], 1)
        self.assertFalse(_judge_provider_real(receipt)["passed"])


if __name__ == "__main__":
    unittest.main()
