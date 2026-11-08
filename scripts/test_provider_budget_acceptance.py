# SPDX-License-Identifier: Apache-2.0
import pytest

from provider_budget_acceptance import require_context_budget


def response(tokens=1000, output=512, field="prompt_tokens"):
    return {"event": "provider.response", "model": "test-model",
            "responseUsage": {field: tokens}, "requestedOutputTokens": output}


@pytest.mark.parametrize("field", ["prompt_tokens", "input_tokens"])
def test_actual_input_budget(field):
    result = require_context_budget([response(field=field)], 4096)
    assert result["responses"] == 1
    assert result["maxInputTokens"] == 1000


@pytest.mark.parametrize("records", [[], [response(4000)], [response(None)],
                                    [response(True)], [response(output=None)],
                                    [response(), response(4000)]])
def test_missing_or_excessive_usage_cannot_pass(records):
    with pytest.raises(ValueError):
        require_context_budget(records, 4096)


def test_metadata_is_not_generation():
    result = require_context_budget([
        {"event": "provider.response", "model": None}, response()
    ], 4096)
    assert result["responses"] == 1
