"""The evidence proxy must not buffer a live provider until EOF."""

import http.client
import importlib.util
import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import tempfile
import threading
import unittest


spec = importlib.util.spec_from_file_location(
    "provider_probe", Path(__file__).resolve().parents[1] / "e2e/provider_probe.py")
probe_module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe_module)


class ProviderProbeStreamingTests(unittest.TestCase):
    def test_opt_in_input_observation_handles_responses_and_message_blocks(self):
        self.assertEqual(probe_module.last_user_text({"input": "direct response input"}), "direct response input")
        self.assertEqual(probe_module.last_user_text({"input": [
            {"role": "user", "content": [{"type": "input_text", "text": "owned marker"},
                {"type": "input_image", "image_url": "private-image"}]},
            {"role": "assistant", "content": "not the user"},
        ]}), "owned marker")
        self.assertEqual(probe_module.last_user_text({"messages": [
            {"role": "user", "content": [{"type": "text", "text": "message input"}]},
        ]}), "message input")
        self.assertIsNone(probe_module.last_user_text({"input": [{"role": "user", "content": [
            {"type": "input_image", "image_url": "private-image"}]}]}))
        self.assertEqual(len(probe_module.last_user_text({"input": "x" * 9000})), 8192)

    def test_final_message_is_not_a_stream_delta(self):
        result = probe_module.inspect_response(
            b'{"choices":[{"message":{"content":"answer","reasoning":"hidden"},"finish_reason":"stop"}]}\n')
        self.assertEqual(result["responseTextBytes"], 6)
        self.assertEqual(result["responseReasoningBytes"], 6)
        self.assertEqual(result["responseTextDeltaCount"], 0)
        self.assertEqual(result["responseReasoningDeltaCount"], 0)

    def test_reasoning_alias_is_counted_without_recording_private_text(self):
        result = probe_module.inspect_response(b'data: {"choices":[{"delta":{"reasoning":"hidden"}}]}\n')
        self.assertEqual(result["responseReasoningBytes"], 6)
        self.assertEqual(result["responseReasoningDeltaCount"], 1)
        self.assertNotIn("hidden", str(result))

    def test_fault_is_opt_in_once_only_and_never_after_terminal(self):
        with tempfile.TemporaryDirectory(prefix="nola-probe-fault-") as directory:
            path = Path(directory) / "trace.jsonl"
            value = probe_module.ResponseEvidence().value
            value["responseTextBytes"] = 100
            self.assertFalse(probe_module.Probe("http://localhost", path).claim_interruption(value))
            probe = probe_module.Probe("http://localhost", path, interrupt_once_after_text_bytes=80)
            value["finishReasons"] = ["stop"]
            self.assertFalse(probe.claim_interruption(value))
            value["finishReasons"] = []
            value["responseDoneMarkerCount"] = 1
            self.assertFalse(probe.claim_interruption(value))
            value["responseDoneMarkerCount"] = 0
            self.assertTrue(probe.claim_interruption(value))
            self.assertFalse(probe.claim_interruption(value))

    def test_long_stream_retains_terminal_evidence_without_retaining_body(self):
        evidence = probe_module.ResponseEvidence()
        event = b'data: {"choices":[{"delta":{"content":"hello"}}]}\n\n'
        for _ in range(20_000):
            evidence.feed(event[:17])
            evidence.feed(event[17:])
            self.assertEqual(len(evidence.pending), 0)
        evidence.feed(b'data: {"choices":[{"delta":{},"finish_reason":"stop"}]}\n\n')
        evidence.feed(b'data: [DONE]\n\n')
        result = evidence.finish()
        self.assertEqual(result["responseTextDeltaCount"], 20_000)
        self.assertEqual(result["responseTextBytes"], 100_000)
        self.assertEqual(result["finishReasons"], ["stop"])
        self.assertEqual(result["responseDoneMarkerCount"], 1)
        self.assertEqual(result["responseOversizedLineCount"], 0)

    def test_oversized_line_is_reported_and_does_not_hide_later_events(self):
        evidence = probe_module.ResponseEvidence()
        for _ in range(100):
            evidence.feed(b"x" * 8192)
            self.assertLessEqual(len(evidence.pending), evidence.MAX_LINE_BYTES)
        evidence.feed(b'\ndata: [DO')
        evidence.feed(b'NE]\r\n')
        result = evidence.finish()
        self.assertEqual(result["responseOversizedLineCount"], 1)
        self.assertEqual(result["responseDoneMarkerCount"], 1)

    def test_small_first_event_arrives_before_upstream_completion(self):
        self._streaming_case(interrupt=False)

    def test_injected_interruption_finishes_http_without_provider_terminal(self):
        self._streaming_case(interrupt=True)

    def test_upstream_timeout_finishes_http_without_provider_terminal(self):
        self._streaming_case(interrupt=False, timeout=True)

    def _streaming_case(self, interrupt, timeout=False):
        first = b'data: {"choices":[{"delta":{"content":"hello"}}]}\n\n'
        last = b'data: [DONE]\n\n'
        release = threading.Event()

        class Upstream(BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"

            def log_message(self, *args):
                pass

            def do_POST(self):
                self.rfile.read(int(self.headers["Content-Length"]))
                self.send_response(200)
                self.send_header("Content-Type", "text/event-stream")
                self.send_header("Content-Length", str(len(first) + len(last)))
                self.end_headers()
                self.wfile.write(first)
                self.wfile.flush()
                release.wait(5)
                self.wfile.write(last)
                self.wfile.flush()

        with tempfile.TemporaryDirectory(prefix="nola-probe-test-") as directory:
            upstream = ThreadingHTTPServer(("127.0.0.1", 0), Upstream)
            proxy = ThreadingHTTPServer(("127.0.0.1", 0), probe_module.Handler)
            proxy.probe = probe_module.Probe(
                f"http://127.0.0.1:{upstream.server_port}", Path(directory) / "trace.jsonl",
                interrupt_once_after_text_bytes=1 if interrupt else 0,
                upstream_timeout_seconds=1 if timeout else 45)
            threads = [threading.Thread(target=server.serve_forever, daemon=True)
                       for server in (upstream, proxy)]
            for thread in threads:
                thread.start()
            client = http.client.HTTPConnection("127.0.0.1", proxy.server_port, timeout=2)
            try:
                client.request("POST", "/v1/chat/completions", body=b'{"stream":true}')
                response = client.getresponse()
                self.assertEqual(response.status, 200)
                # The upstream cannot finish until we observe this event.
                # read(8192) in the proxy deadlocks here until client timeout.
                self.assertEqual(response.read1(8192), first)
                if interrupt or timeout:
                    # No upstream EOF yet: the fault must close valid HTTP
                    # framing promptly, leaving only the SSE terminal absent.
                    self.assertEqual(response.read(), b"")
                    self.assertEqual(proxy.probe.interruption_claimed, interrupt)
                    release.set()
                else:
                    release.set()
                    self.assertEqual(response.read(), last)
            finally:
                release.set()
                client.close()
                for server in (proxy, upstream):
                    server.shutdown()
                    server.server_close()
                for thread in threads:
                    thread.join(timeout=3)
            if timeout:
                evidence = json.loads((Path(directory) / "trace.jsonl").read_text().splitlines()[-1])
                self.assertEqual(evidence["errorStage"], "upstream_body")
                self.assertIn("TimeoutError", evidence["error"])
                self.assertEqual(evidence["responseDoneMarkerCount"], 0)


if __name__ == "__main__":
    unittest.main()
