# SPDX-License-Identifier: Apache-2.0
"""Transport evidence gate, separate from canonical-answer correctness."""


def completed_stream(record):
    reasons = record.get("finishReasons")
    if not isinstance(reasons, list) or not reasons:
        return False
    if record.get("responseOversizedLineCount") != 0:
        return False
    # Responses has a typed terminal event; Chat Completions additionally
    # requires its stream terminator. Length/incomplete is not completion.
    if reasons == ["completed"]:
        return True
    return (all(reason in ("stop", "tool_calls", "function_call") for reason in reasons)
            and type(record.get("responseDoneMarkerCount")) is int
            and record["responseDoneMarkerCount"] > 0)


def require_text_stream(records):
    """Require observed successful streaming text, never infer it from tools."""
    qualifying = [record for record in records
                  if record.get("event") == "provider.response"
                  and record.get("stream") is True
                  and isinstance(record.get("status"), int)
                  and 200 <= record["status"] < 300
                  and not record.get("error")
                  and not record.get("injectedInterruption")
                  and completed_stream(record)
                  and isinstance(record.get("responseTextDeltaCount"), int)
                  and record["responseTextDeltaCount"] > 0
                  and isinstance(record.get("responseTextBytes"), int)
                  and record["responseTextBytes"] > 0]
    if not qualifying:
        raise ValueError("No successful provider text stream observed; canonical answer or tool output is not text-stream evidence")
    return len(qualifying)
