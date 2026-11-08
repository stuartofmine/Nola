#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Validate an explicit, route-bound real-test declaration without secrets.

This fixture is an opt-in test contract, not capability detection or a change
to the user's model configuration. False declarations stay false.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path


class FixtureError(ValueError):
    pass


RUNTIME_FLAGS = {"native_tool_calls", "parallel_tool_calls", "reasoning", "reasoning_continuity",
                 "vision_input", "request_continuation", "stream_text", "stream_reasoning", "stream_tool_calls"}


def configured_runtime_declaration(config: dict) -> dict | None:
    llm = config.get("llm", {})
    runtime = llm.get("capabilities", config.get("model_capabilities"))
    if runtime is None:
        return None
    if (not isinstance(runtime, dict) or set(runtime) - RUNTIME_FLAGS - {"instruction_profile", "max_request_continuations"}
            or any(type(runtime[key]) is not bool for key in set(runtime) & RUNTIME_FLAGS)):
        raise FixtureError("invalid runtime execution declaration")
    if "max_request_continuations" in runtime and (
        type(runtime["max_request_continuations"]) is not int or not 0 <= runtime["max_request_continuations"] <= 255
    ):
        raise FixtureError("invalid request continuation policy")
    if "instruction_profile" in runtime and not isinstance(runtime["instruction_profile"], str):
        raise FixtureError("invalid instruction profile")
    return runtime


def strict_object(raw: str) -> dict:
    def unique(pairs: list[tuple[str, object]]) -> dict:
        result = {}
        for key, value in pairs:
            if key in result:
                raise FixtureError("duplicate declaration field")
            result[key] = value
        return result

    try:
        value = json.loads(raw, object_pairs_hook=unique)
    except json.JSONDecodeError as error:
        raise FixtureError("invalid declaration JSON") from error
    if not isinstance(value, dict):
        raise FixtureError("declaration must be an object")
    return value


def fixture(config: dict, environment: dict[str, str], *, model: str | None = None,
            endpoint: str | None = None, require_tool: bool = True) -> dict:
    llm = config.get("llm")
    if not isinstance(llm, dict):
        raise FixtureError("configuration must contain llm")
    selected_model = model if model is not None else llm.get("model")
    if not isinstance(selected_model, str) or not selected_model.strip():
        raise FixtureError("model is missing")
    raw = environment.get("NOLA_REAL_E2E_EXECUTION_CAPABILITIES_JSON")
    if raw is not None:
        declaration = strict_object(raw)
    else:
        if selected_model != llm.get("model") or (endpoint is not None and endpoint != llm.get("endpoint")):
            raise FixtureError("an overridden route requires an explicit test declaration")
        declaration = llm.get("executionCapabilities")
        if declaration is None:
            runtime = configured_runtime_declaration(config)
            if isinstance(runtime, dict) and all(type(runtime.get(key)) is bool for key in (
                "native_tool_calls", "vision_input", "reasoning",
            )):
                declaration = {
                    "toolCalls": runtime["native_tool_calls"], "imageInput": runtime["vision_input"],
                    "reasoning": runtime["reasoning"], "samplingParameters": False,
                }
            else:
                raise FixtureError("explicit execution capabilities are missing; set NOLA_REAL_E2E_EXECUTION_CAPABILITIES_JSON")
    if not isinstance(declaration, dict):
        raise FixtureError("execution capabilities must be an object")
    required = {"toolCalls", "imageInput", "reasoning", "samplingParameters"}
    optional = {"samplingRequiresNoReasoning", "chatOutputTokenField"}
    if not required.issubset(declaration) or set(declaration) - required - optional:
        raise FixtureError("unknown or missing execution capability fields")
    execution = dict(declaration)
    execution.setdefault("samplingRequiresNoReasoning", False)
    if any(type(execution[key]) is not bool for key in required | {"samplingRequiresNoReasoning"}):
        raise FixtureError("execution capability flags must be booleans")
    field = execution.get("chatOutputTokenField")
    if field is not None and (not isinstance(field, str) or field not in {"auto", "max_tokens", "max_completion_tokens"}):
        raise FixtureError("unsupported output token field")
    if execution["samplingRequiresNoReasoning"] and not (execution["samplingParameters"] and execution["reasoning"]):
        raise FixtureError("conditional sampling requires sampling and reasoning support")
    if require_tool and not execution["toolCalls"]:
        raise FixtureError("tool acceptance contradicts an explicitly disabled tool capability")
    if environment.get("NOLA_MAC_REAL_VISION_INPUT", "0") == "1" and not execution["imageInput"]:
        raise FixtureError("vision acceptance contradicts an explicitly disabled image capability")
    if environment.get("NOLA_MAC_REAL_REQUIRE_THINKING", "0") == "1" and not execution["reasoning"]:
        raise FixtureError("thinking acceptance contradicts an explicitly disabled reasoning capability")
    protocol = environment.get("NOLA_REAL_E2E_MODEL_PROTOCOL", llm.get("wireProtocol"))
    if protocol is None:
        # Preserve the configured product default; never examine model names or
        # endpoint strings to choose a protocol or declare a capability.
        provider = llm.get("provider")
        protocol = {None: "chat_completions", "openai": "chat_completions",
                    "openai-compatible": "chat_completions", "openai-responses": "responses",
                    "anthropic": "anthropic"}.get(provider)
    if protocol not in {"chat_completions", "responses", "anthropic"}:
        raise FixtureError("explicit supported model protocol is required")
    return {"model": selected_model, "wireProtocol": protocol, "executionCapabilities": execution}


def runtime_capabilities(value: dict, explicit_runtime: dict | None = None) -> dict:
    execution = value["executionCapabilities"]
    # These are this harness's frozen execution policy, not additional provider
    # discoveries. It chooses streaming text, no parallelism or continuation.
    result = {
        "native_tool_calls": execution["toolCalls"], "parallel_tool_calls": False,
        "vision_input": execution["imageInput"], "reasoning": execution["reasoning"],
        "reasoning_continuity": value["wireProtocol"] == "responses" and execution["reasoning"],
        "stream_text": True, "stream_reasoning": execution["reasoning"],
        "stream_tool_calls": execution["toolCalls"], "request_continuation": False,
        "max_request_continuations": 1,
    }
    if explicit_runtime is not None:
        result.update(explicit_runtime)
    return result


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--config", required=True, type=Path)
    parser.add_argument("--model")
    parser.add_argument("--endpoint")
    parser.add_argument("--require-tool", choices=("0", "1"), default="1")
    parser.add_argument("--format", choices=("mac", "cli"), default="mac")
    arguments = parser.parse_args()
    try:
        config = strict_object(arguments.config.read_text(encoding="utf-8"))
        environment = dict(os.environ)
        value = fixture(config, environment,
                        model=arguments.model, endpoint=arguments.endpoint,
                        require_tool=arguments.require_tool == "1")
        if arguments.format == "cli":
            runtime = (configured_runtime_declaration(config)
                       if "NOLA_REAL_E2E_EXECUTION_CAPABILITIES_JSON" not in environment
                       and config["llm"].get("executionCapabilities") is None else None)
            value["runtimeCapabilities"] = runtime_capabilities(value, runtime)
            value["provider"] = {"chat_completions": "openai-compatible", "responses": "openai-responses",
                                 "anthropic": "anthropic"}[value["wireProtocol"]]
        print(json.dumps(value, ensure_ascii=False, separators=(",", ":")))
        return 0
    except (OSError, ValueError, TypeError):
        # Neither raw config, model route credentials, nor malformed JSON is
        # interpolated into diagnostics. Callers must fix the declaration.
        print("Invalid or missing explicit real-test execution declaration; confirm fixture capabilities and protocol.", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
