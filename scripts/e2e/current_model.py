# SPDX-License-Identifier: Apache-2.0
"""Resolve the declared default route without guessing providers or capabilities."""

from urllib.parse import urlsplit


def selected_model(config: dict, environment: dict) -> dict:
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
        key = environment.get(connection.get("api_key_env", ""), "")
        if not key:
            raise ValueError("Default model credential environment is unavailable")
        result = {"endpoint": endpoint, "model": models[0]["id"], "api_key": key, "wireProtocol": protocol}
        # Only the selected model's explicit declarations apply to this route.
        for name in ("capabilities", "executionCapabilities"):
            if name in models[0]:
                result[name] = models[0][name]
        return result
    except (KeyError, TypeError, AttributeError) as error:
        raise ValueError("Invalid current model configuration") from error
