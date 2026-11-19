# SPDX-License-Identifier: Apache-2.0
"""Resolve the declared default route without guessing providers or capabilities."""

from urllib.parse import urlsplit


def declared_route(config: dict) -> tuple[dict, dict]:
    try:
        llm = config["llm"]
        selected = llm["default_model"]
        connections = [item for item in llm["connections"] if item["id"] == selected["connection"]]
        if len(connections) != 1:
            raise ValueError("Default model connection is missing or ambiguous")
        connection = connections[0]
        models = [item for item in connection["models"] if item["id"] == selected["model"]]
        if len(models) != 1:
            raise ValueError("Default model is missing or ambiguous")
        protocol = connection["protocol"]
        if protocol not in {"chat_completions", "responses", "anthropic"}:
            raise ValueError("Default model protocol is unsupported")
        endpoint = connection["endpoint"]
        parsed = urlsplit(endpoint)
        if parsed.scheme not in {"http", "https"} or not parsed.hostname or parsed.username or parsed.password:
            raise ValueError("Model endpoint must be HTTP(S) without embedded credentials")
        return connection, models[0]
    except (KeyError, TypeError, AttributeError) as error:
        raise ValueError("Invalid current model configuration") from error


def selected_model(config: dict, environment: dict) -> dict:
    connection, model = declared_route(config)
    key = environment.get(connection.get("api_key_env", ""), "")
    if not key:
        raise ValueError("Default model credential environment is unavailable")
    result = {"endpoint": connection["endpoint"], "model": model["id"], "api_key": key, "wireProtocol": connection["protocol"]}
    for name in ("capabilities", "executionCapabilities"):
        if name in model:
            result[name] = model[name]
    return result
