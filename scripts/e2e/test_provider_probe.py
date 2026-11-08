import importlib.util
import contextlib
import http.client
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import hashlib
from pathlib import Path
import threading

from e2e.provider_fault_relay import AuthenticationFaultRelay, ContextLimitFaultRelay


MODULE_PATH = Path(__file__).with_name("provider_probe.py")
SPEC = importlib.util.spec_from_file_location("provider_probe", MODULE_PATH)
PROBE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(PROBE)


def test_instruction_observer_matches_exact_prompt_bytes_without_logging_text(tmp_path):
    expected = {"templateContent": "模板 exact", "styleInstruction": "style\nexact", "customInstruction": "custom exact"}
    fixture = tmp_path / "expected.json"
    fixture.write_text(json.dumps(expected))
    request = {"messages": [{"role": "system", "content": "prefix " + expected["templateContent"]}],
               "instructions": expected["styleInstruction"], "input": [{"role": "user", "content": [{"type": "input_text", "text": expected["customInstruction"]}]}],
               "metadata": {"secret": "must-not-leak"}}
    result = PROBE.inspect_expected_instructions(request, fixture)
    assert result["requestInstructionObservation"] == "observed"
    for key, value in expected.items():
        assert result["requestInstructionMatches"][key] == {"present": True, "sha256": hashlib.sha256(value.encode()).hexdigest()}
        assert value not in json.dumps(result, ensure_ascii=False)
    assert "must-not-leak" not in json.dumps(result)


def test_instruction_observer_does_not_match_metadata_or_normalize_missing_bytes(tmp_path):
    fixture = tmp_path / "expected.json"
    fixture.write_text(json.dumps({"templateContent": "exact\nbytes", "styleInstruction": "style", "customInstruction": "custom"}))
    request = {"metadata": {"content": "exact\nbytes style custom"}, "messages": [{"role": "system", "content": "exact bytes"}]}
    result = PROBE.inspect_expected_instructions(request, fixture)
    assert not any(value["present"] for value in result["requestInstructionMatches"].values())
    assert PROBE.inspect_expected_instructions(request, tmp_path / "missing.json")["requestInstructionObservation"] == "expected_instructions_unavailable"
    assert PROBE.inspect_expected_instructions(request, None) == {}


def test_schema_metrics_are_numeric_and_explicitly_estimated():
    assert PROBE.tool_schema_metrics({})["toolSchemaEstimatedTokens"] == 0
    tools = [{"name":"秘密", "description":"do not leak me"}]
    result = PROBE.tool_schema_metrics({"tools":tools})
    text = json.dumps(tools[0], ensure_ascii=False, separators=(",", ":"))
    ascii_count = sum(ord(char) < 128 for char in text)
    assert result["toolSchemaUtf8Bytes"] == len(text.encode("utf-8"))
    assert result["toolSchemaEstimatedTokens"] == (ascii_count + 3) // 4 + 2
    assert result["toolSchemaTokenMethod"] == "ascii4_nonascii1_json"
    assert "leak" not in json.dumps(result)
    assert PROBE.tool_schema_metrics({"tools":None})["toolSchemaEstimatedTokens"] is None


@contextlib.contextmanager
def owned_upstream():
    received = []
    class Upstream(BaseHTTPRequestHandler):
        def log_message(self, *_arguments):
            pass

        def do_POST(self):
            body = self.rfile.read(int(self.headers["Content-Length"]))
            received.append({"path":self.path,"body":body,"authorization":self.headers.get("Authorization")})
            response = b'{"owned_upstream":true}'
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(response)))
            self.end_headers()
            self.wfile.write(response)
    server = ThreadingHTTPServer(("127.0.0.1", 0), Upstream)
    worker = threading.Thread(target=server.serve_forever, kwargs={"poll_interval":0.05})
    worker.start()
    try:
        yield f"http://127.0.0.1:{server.server_port}", received
    finally:
        server.shutdown()
        server.server_close()
        worker.join(timeout=3)
        assert not worker.is_alive()


def relay_post(relay, path, value, credential="owned-valid-fixture"):
    connection = http.client.HTTPConnection("127.0.0.1", relay.server.server_port, timeout=3)
    try:
        connection.request("POST", path, json.dumps(value),
            {"Authorization":"Bearer " + credential,"Content-Type":"application/json"})
        response = connection.getresponse()
        return response.status, response.read()
    finally:
        connection.close()


def test_authentication_fault_uses_protocol_generation_input_then_forwards_exact_request(tmp_path):
    for protocol, suffix, field in (("chat_completions","/chat/completions","messages"),
            ("responses","/responses","input"),("anthropic","/messages","messages")):
        for full_endpoint in (False, True):
            with owned_upstream() as (origin, received):
                path = "/v1" + suffix
                upstream = origin + (path if full_endpoint else "/v1")
                log = tmp_path / f"{protocol}-{full_endpoint}.ndjson"
                with AuthenticationFaultRelay(upstream, log, protocol, "owned-model") as relay:
                    request = {"model":"owned-model",field:[{"role":"user","content":"same input"}]}
                    # No prompt-text trigger; metadata, empty input and other
                    # model routes cannot consume the one generation fault.
                    assert relay_post(relay,"/v1/tokenize",request)[0] == 200
                    assert relay_post(relay,path,{"model":"owned-model",field:[]})[0] == 200
                    assert relay_post(relay,path,{**request,"model":"another-model"})[0] == 200
                    assert relay.probe.injected == 0
                    before = len(received)
                    status, body = relay_post(relay,path,request)
                    assert status == 401 and json.loads(body)["error"]["type"] == "authentication_error"
                    assert len(received) == before
                    status, body = relay_post(relay,path,request)
                    assert status == 200 and body == b'{"owned_upstream":true}'
                    assert len(received) == before + 1
                    assert json.loads(received[-1]["body"]) == request
                    assert received[-1]["path"] == path
                    assert received[-1]["authorization"] == "Bearer owned-valid-fixture"
                relay.assert_recovered()
                events = [json.loads(line) for line in log.read_text().splitlines()]
                faults = [event for event in events if event.get("injectedHttpError")]
                assert len(faults) == 1 and faults[0]["forwarded"] is False


def test_debug_credential_cannot_leave_relay_even_on_discovery_or_retry(tmp_path):
    secret = "synthetic-debug-key"
    with owned_upstream() as (origin, received):
        log = tmp_path / "debug.ndjson"
        with AuthenticationFaultRelay(origin + "/v1",log,"chat_completions","owned-model",secret) as relay:
            request = {"model":"owned-model","messages":[{"role":"user","content":"generation"}]}
            assert relay_post(relay,"/v1/tokenize",{},secret)[0] == 401
            assert relay.probe.injected == 0 and not received
            assert relay_post(relay,"/v1/chat/completions",request,secret)[0] == 401
            assert relay_post(relay,"/v1/chat/completions",request,secret)[0] == 401
            assert relay.probe.injected == 1 and not received
            assert relay_post(relay,"/v1/chat/completions",request)[0] == 200
        assert len(received) == 1 and secret not in json.dumps(received,default=lambda value:value.decode())
        assert secret not in log.read_text()
        # An unexpected repeated generation failure is not silently accepted.
        try:
            relay.assert_recovered()
        except AssertionError:
            pass
        else:
            raise AssertionError("multiple rejected generations were not detected")


def test_context_limit_fault_is_armed_after_history_then_injects_once_and_preserves_request(tmp_path):
    for protocol, suffix, field in (("chat_completions","/chat/completions","messages"),
            ("responses","/responses","input"),("anthropic","/messages","messages")):
        with owned_upstream() as (origin, received):
            path = "/v1" + suffix
            with ContextLimitFaultRelay(origin + "/v1", tmp_path / f"context-{protocol}.ndjson",
                    protocol, "owned-model") as relay:
                request = {"model":"owned-model",field:[{"role":"user","content":"current instruction"}]}
                # The completed seed must use this exact endpoint without
                # consuming the later, explicitly armed generation fault.
                assert relay_post(relay,path,request)[0] == 200
                assert relay.probe.injected == 0
                relay.arm()
                assert relay_post(relay,"/v1/tokenize",request)[0] == 200
                assert relay_post(relay,path,{"model":"owned-model",field:[]})[0] == 200
                assert relay_post(relay,path,{**request,"model":"foreign-model"})[0] == 200
                assert relay.probe.injected == 0
                count = len(received)
                status, raw = relay_post(relay,path,request)
                assert status == 400 and json.loads(raw)["error"]["code"] == "context_length_exceeded"
                assert len(received) == count
                assert relay_post(relay,path,request)[0] == 200
                assert json.loads(received[-1]["body"]) == request
                assert received[-1]["path"] == path
                relay.assert_recovered()
                try:
                    relay.arm()
                except AssertionError:
                    pass
                else:
                    raise AssertionError("fault can be armed twice")
            faults = [json.loads(line) for line in relay.probe.log_path.read_text().splitlines()
                if json.loads(line).get("injectedHttpError")]
            assert len(faults) == 1 and faults[0]["status"] == 400 and faults[0]["forwarded"] is False


def test_responses_string_input_is_generation_but_blank_input_is_not(tmp_path):
    with owned_upstream() as (origin, received):
        with AuthenticationFaultRelay(origin + "/v1",tmp_path / "responses-string.ndjson",
                "responses","owned-model") as relay:
            assert relay_post(relay,"/v1/responses",{"model":"owned-model","input":"  "})[0] == 200
            assert relay.probe.injected == 0
            assert relay_post(relay,"/v1/responses",{"model":"owned-model","input":"actual input"})[0] == 401
        assert len(received) == 1


def test_probe_preserves_full_chat_endpoint_shape():
    probe = PROBE.Probe(
        "https://api.example.test/v1/chat/completions",
        Path("/tmp/unused-provider-probe.ndjson"),
    )
    assert probe.upstream.path == "/v1/chat/completions"


def test_chat_usage_preserves_cache_counters_and_streamed_text():
    usage = {"prompt_tokens": 100, "completion_tokens": 5,
             "prompt_tokens_details": {"cached_tokens": 80, "secret": "omit"}}
    collector = PROBE.ResponseEvidence()
    for event in [
        {"choices": [{"delta": {"content": "one"}}]},
        {"choices": [{"delta": {"content": "two"}}], "usage": usage},
        {"choices": [], "usage": usage},
    ]:
        collector.feed(b"data: " + json.dumps(event).encode() + b"\n")
    evidence = collector.finish()
    assert evidence["responseTextBytes"] == 6
    assert evidence["responseUsage"] == {
        "prompt_tokens": 100, "completion_tokens": 5,
        "prompt_tokens_details": {"cached_tokens": 80},
    }


def test_absent_or_invalid_usage_is_unknown_not_zero():
    for usage in (None, {}, {"input_tokens": True}, {"prompt_tokens": -1}):
        assert PROBE.usage_counters(usage) is None
    assert PROBE.usage_counters({"input_tokens_details": {"cached_tokens": 0}}) == {
        "input_tokens_details": {"cached_tokens": 0}}


def test_output_limit_matches_each_wire_protocol():
    for field in ("max_output_tokens", "max_completion_tokens", "max_tokens"):
        assert PROBE.output_limit({field: 5600}) == 5600
    assert PROBE.output_limit({}) is None
    assert PROBE.output_limit({"max_tokens": True}) is None


def test_probe_preserves_base_endpoint_shape():
    probe = PROBE.Probe(
        "https://api.example.test/compatible-mode/v1",
        Path("/tmp/unused-provider-probe.ndjson"),
    )
    assert probe.upstream.path == "/compatible-mode/v1"


def test_truncation_is_opt_in_single_cold_envelope_and_does_not_mutate_request(tmp_path):
    request = {"input": [{"role": "user", "content": [{"type": "input_text", "text": json.dumps({
        "previous_checkpoint": "", "next_archive_segment": "fixture"
    })}]}], "reasoning": {"effort": "none"}, "max_output_tokens": 16384}
    disabled = PROBE.Probe("http://localhost/v1", tmp_path / "disabled")
    assert not disabled.claim_compaction_truncation(request)
    probe = PROBE.Probe("http://localhost/v1", tmp_path / "enabled", truncate_compaction_once_tokens=128)
    assert not probe.claim_compaction_truncation({**request, "tools": [{"name": "read_file"}]})
    assert not probe.claim_compaction_truncation({**request, "reasoning": {"effort": "high"}})
    assert not probe.claim_compaction_truncation({**request, "input": [{"role": "user", "content": "Summarize this"}]})
    assert probe.claim_compaction_truncation(request)
    assert not probe.claim_compaction_truncation(request)
    assert request["max_output_tokens"] == 16384

    hot = {**request, "tools": [{"name": "read_file"}], "input": [
        {"role": "user", "content": "earlier source"},
        {"role": "user", "content": "Context maintenance only. Do not execute any task or call any tool. Return only the checkpoint text. fixture"}]}
    probe = PROBE.Probe("http://localhost/v1", tmp_path / "hot", truncate_compaction_once_tokens=128)
    assert probe.claim_compaction_truncation(hot)
    assert not probe.claim_compaction_truncation(hot)


def test_probe_bounds_upstream_wait_and_rejects_nonpositive_timeout():
    default_probe = PROBE.Probe(
        "https://api.example.test/v1",
        Path("/tmp/unused-provider-probe-default.ndjson"),
    )
    bounded_probe = PROBE.Probe(
        "https://api.example.test/v1",
        Path("/tmp/unused-provider-probe-bounded.ndjson"),
        upstream_timeout_seconds=0,
    )
    assert default_probe.upstream_timeout_seconds == 45.0
    assert bounded_probe.upstream_timeout_seconds == 1.0


def test_response_inspector_records_tool_protocol_without_model_text():
    body = b"\n".join([
        b'data: {"choices":[{"delta":{"reasoning_content":"think"}}]}',
        b'data: {"choices":[{"delta":{"tool_calls":[{"function":{"name":"skill_invoke","arguments":"{\\"skill\\":\\"demo\\"}"}}]},"finish_reason":"tool_calls"}]}',
        b'data: [DONE]',
    ])

    evidence = PROBE.inspect_response(body)

    assert evidence["responseEventCount"] == 2
    assert evidence["responseTextBytes"] == 0
    assert evidence["responseReasoningBytes"] == 5
    assert evidence["responseToolNames"] == ["skill_invoke"]
    assert evidence["responseToolArgumentBytes"] > 0
    assert evidence["finishReasons"] == ["tool_calls"]


def test_responses_probe_counts_reasoning_and_terminal_usage_without_double_counting():
    events = [
        {"type": "response.reasoning_text.delta", "delta": "think"},
        {"type": "response.output_text.delta", "delta": "summary"},
        {"type": "response.incomplete", "response": {
            "model": "owned-dynamic-route-model",
            "incomplete_details": {"reason": "max_output_tokens"},
            "usage": {"input_tokens": 100, "output_tokens": 20, "secret": "private"},
            "output": [
                {"type": "reasoning", "content": [{"text": "think"}]},
                {"type": "message", "content": [{"text": "summary"}]},
            ],
        }},
    ]
    encode = lambda items: b"\n".join(b"data: " + json.dumps(item).encode() for item in items)
    evidence = PROBE.inspect_response(encode(events))
    assert evidence["responseTextBytes"] == 7
    assert evidence["responseReasoningBytes"] == 5
    assert evidence["finishReasons"] == ["max_output_tokens"]
    assert evidence["responseUsage"] == {"input_tokens": 100, "output_tokens": 20}
    assert evidence["responseModels"] == ["owned-dynamic-route-model"]
    terminal_only = PROBE.inspect_response(encode(events[-1:]))
    assert terminal_only["responseTextBytes"] == 7
    assert terminal_only["responseReasoningBytes"] == 5
    collector = PROBE.ResponseEvidence()
    wire = encode(events)
    for index in range(0, len(wire), 7):
        collector.feed(wire[index:index+7])
    streamed = collector.finish()
    assert streamed["responseModels"] == ["owned-dynamic-route-model"]
    for key, value in evidence.items():
        assert streamed[key] == value, key


def test_ui_action_observer_records_revision_without_form_values():
    envelope = {"type": "ui_action", "surface_revision": 3,
                "message": {"version": "v0.9.1", "action": {"context": {"name": "not-for-logs"}}}}
    request = {"input": [{"role": "user", "content": [{"type": "input_text", "text": json.dumps(envelope)}]}]}
    observed = PROBE.inspect_ui_action_revisions(request)
    assert observed == {"modelUiActionSurfaceRevisions": [3], "modelUiActionProtocolValid": [True]}
    assert "not-for-logs" not in json.dumps(observed)
    del envelope["surface_revision"]
    request["input"][0]["content"][0]["text"] = json.dumps(envelope)
    assert PROBE.inspect_ui_action_revisions(request)["modelUiActionSurfaceRevisions"] == [None]
