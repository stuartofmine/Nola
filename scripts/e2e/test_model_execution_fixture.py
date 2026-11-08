from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

import pytest

from model_execution_fixture import FixtureError, configured_runtime_declaration, fixture, runtime_capabilities, strict_object


def config() -> dict:
    return {"llm": {"endpoint": "https://route.invalid/v1", "model": "arbitrary-model", "api_key": "SECRET-NOT-PRINTED"}}


def declaration(**updates: object) -> dict:
    value = {"toolCalls": True, "imageInput": False, "reasoning": False, "samplingParameters": False}
    value.update(updates)
    return value


def environment(**updates: object) -> dict:
    return {"NOLA_REAL_E2E_EXECUTION_CAPABILITIES_JSON": json.dumps(declaration(**updates))}


def test_explicit_fixture_is_exact_and_secret_free() -> None:
    value = fixture(config(), environment())
    assert value["model"] == "arbitrary-model"
    assert value["wireProtocol"] == "chat_completions"
    assert value["executionCapabilities"]["toolCalls"] is True
    assert value["executionCapabilities"]["reasoning"] is False
    assert "SECRET" not in json.dumps(value)
    assert "route.invalid" not in json.dumps(value)


def test_missing_capabilities_cannot_be_inferred_from_name_or_endpoint() -> None:
    value = config()
    value["llm"].update(model="gpt-5-tools", endpoint="https://api.openai.com/v1")
    with pytest.raises(FixtureError, match="missing"):
        fixture(value, {})


def test_false_values_survive_without_truthy_defaults() -> None:
    value = fixture(config(), environment(toolCalls=False), require_tool=False)
    runtime = runtime_capabilities(value)
    assert runtime["native_tool_calls"] is False
    assert runtime["stream_tool_calls"] is False
    assert runtime["reasoning"] is False
    assert runtime["vision_input"] is False


@pytest.mark.parametrize("changes", [
    {"toolCalls": False}, {"toolCalls": "true"}, {"reasoning": 1}, {"reasoning": None},
    {"unsupported": True}, {"native_tool_calls": True}, {"samplingRequiresNoReasoning": True},
    {"chatOutputTokenField": {}}, {"chatOutputTokenField": "made-up"},
])
def test_invalid_or_contradictory_execution_contract_fails(changes: dict) -> None:
    with pytest.raises(FixtureError):
        fixture(config(), environment(**changes))


@pytest.mark.parametrize("raw", ['{"toolCalls":true,"toolCalls":false}', '{"toolCalls":true,"\\u0074oolCalls":false}', '[]', '{bad'])
def test_duplicate_and_invalid_json_fails(raw: str) -> None:
    with pytest.raises(FixtureError):
        strict_object(raw)


@pytest.mark.parametrize("requirement", ["NOLA_MAC_REAL_VISION_INPUT", "NOLA_MAC_REAL_REQUIRE_THINKING"])
def test_test_requirements_do_not_enable_disabled_capabilities(requirement: str) -> None:
    value = environment()
    value[requirement] = "1"
    with pytest.raises(FixtureError, match="contradicts"):
        fixture(config(), value)


def test_configured_declaration_cannot_move_to_another_route_without_opt_in() -> None:
    source = config()
    source["llm"]["executionCapabilities"] = declaration()
    assert fixture(source, {})["executionCapabilities"]["toolCalls"] is True
    with pytest.raises(FixtureError, match="overridden route"):
        fixture(source, {}, model="replacement")
    with pytest.raises(FixtureError, match="overridden route"):
        fixture(source, {}, endpoint="https://other.invalid/v1")
    assert fixture(source, environment(), model="replacement")["model"] == "replacement"


def test_protocol_is_explicit_not_guessed_from_url_or_model() -> None:
    source = config()
    source["llm"].update(endpoint="https://responses.invalid/responses", model="responses-model")
    assert fixture(source, environment())["wireProtocol"] == "chat_completions"
    selected = environment()
    selected["NOLA_REAL_E2E_MODEL_PROTOCOL"] = "responses"
    assert fixture(source, selected)["wireProtocol"] == "responses"
    selected["NOLA_REAL_E2E_MODEL_PROTOCOL"] = "unknown"
    with pytest.raises(FixtureError):
        fixture(source, selected)


def test_existing_runtime_declared_false_stream_flags_are_not_overwritten() -> None:
    source = config()
    source["llm"]["capabilities"] = {
        "native_tool_calls": True, "vision_input": False, "reasoning": False,
        "stream_tool_calls": False, "stream_text": False,
    }
    value = fixture(source, {})
    runtime = runtime_capabilities(value, configured_runtime_declaration(source))
    assert runtime["native_tool_calls"] is True
    assert runtime["stream_tool_calls"] is False
    assert runtime["stream_text"] is False


def test_false_runtime_tool_declaration_is_not_replaced_with_true() -> None:
    source = config()
    source["llm"]["capabilities"] = {"native_tool_calls": False, "vision_input": False, "reasoning": False}
    with pytest.raises(FixtureError, match="disabled tool"):
        fixture(source, {})


def test_configured_runtime_alias_is_not_silently_ignored() -> None:
    source = config()
    source["llm"]["capabilities"] = {"native_tool_calls": True, "vision_input": False, "reasoning": False, "nativeToolCalls": False}
    with pytest.raises(FixtureError, match="invalid runtime"):
        fixture(source, {})


def test_command_failure_does_not_print_secret_config(tmp_path: Path) -> None:
    path = tmp_path / "config.json"
    path.write_text(json.dumps(config()), encoding="utf-8")
    script = Path(__file__).with_name("model_execution_fixture.py")
    result = subprocess.run([sys.executable, str(script), "--config", str(path)],
                            env={}, text=True, capture_output=True, check=False)
    assert result.returncode == 2
    assert "SECRET-NOT-PRINTED" not in result.stdout + result.stderr
    assert "arbitrary-model" not in result.stdout + result.stderr
