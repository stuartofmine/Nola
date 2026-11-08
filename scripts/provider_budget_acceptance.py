# SPDX-License-Identifier: Apache-2.0
"""Verify actual provider input usage against the admitted context window."""


def require_context_budget(records, window):
    if type(window) is not int or window <= 0:
        raise ValueError("context window must be a positive integer")
    safety = min(8192, max(512, window // 20))
    responses = [row for row in records
                 if row.get("event") == "provider.response" and row.get("model")]
    if not responses:
        raise ValueError("no model responses observed")
    peak = 0
    for index, row in enumerate(responses):
        usage = row.get("responseUsage") or {}
        tokens = usage.get("input_tokens", usage.get("prompt_tokens"))
        output = row.get("requestedOutputTokens")
        if (type(tokens) is not int or tokens < 0
                or type(output) is not int or output <= 0):
            raise ValueError(f"response {index}: missing valid input/output budget evidence")
        if tokens + output + safety > window:
            raise ValueError(f"response {index}: {tokens} input + {output} output + "
                             f"{safety} safety exceeds {window}")
        peak = max(peak, tokens)
    return {"responses": len(responses), "maxInputTokens": peak,
            "contextWindow": window, "safetyTokens": safety}
