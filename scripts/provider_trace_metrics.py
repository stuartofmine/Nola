"""Observed transport counts, not Run success or a complete submission denominator."""

import json
from pathlib import Path
from urllib.parse import urlsplit

from provider_stream_acceptance import completed_stream


def provider_trace_metrics(paths):
    metrics = dict(successfulRequests=0, successfulModelRequests=0,
                   observedModelRequests=0, completedModelStreams=0,
                   malformedRows=0, unreadableFiles=0)
    hosts = set()
    model_hosts = set()
    # Multiple assertions may reference the same file. Do not count it twice.
    for path in sorted({Path(path).resolve() for path in paths}):
        try:
            lines = path.read_text(encoding="utf-8").splitlines()
        except (OSError, UnicodeError):
            metrics["unreadableFiles"] += 1
            continue
        for line in lines:
            if not line.strip():
                continue
            try:
                row = json.loads(line)
                if not isinstance(row, dict):
                    raise ValueError("trace row is not an object")
                event = row.get("event")
                if event not in ("provider.response", "external_executor.response"):
                    continue
                status, size = row.get("status"), row.get("responseBytes")
                # A failed connection may have no HTTP status; it is still an observation.
                if status is not None and (type(status) is not int or not 100 <= status <= 599):
                    raise ValueError("invalid HTTP status")
                if type(size) is not int or size < 0:
                    raise ValueError("invalid response size")
                route = urlsplit(str(row.get("upstreamPath") or "")).path.rstrip("/")
                model = row.get("model")
                is_model = (
                    event == "provider.response" and isinstance(model, str) and bool(model.strip())
                    and route.endswith(("/chat/completions", "/responses", "/messages"))
                ) or (
                    event == "external_executor.response" and bool(row.get("backend"))
                    and bool(row.get("sessionId")) and row.get("structuredEvents") is True
                )
            except (ValueError, TypeError):
                metrics["malformedRows"] += 1
                continue
            host = str(row.get("upstreamHost") or "").strip().lower()
            if host:
                hosts.add(host)
            metrics["observedModelRequests"] += int(bool(is_model))
            # HTTP success is not proof of a complete stream or a successful task.
            success = status is not None and 200 <= status < 300 and size > 0
            metrics["successfulRequests"] += int(success)
            metrics["successfulModelRequests"] += int(success and bool(is_model))
            if success and is_model:
                if host:
                    model_hosts.add(host)
                if (row.get("stream") is True and not row.get("error")
                        and not row.get("injectedInterruption") and completed_stream(row)):
                    metrics["completedModelStreams"] += 1
    return {**metrics, "upstreamHosts": sorted(hosts),
            "successfulModelUpstreamHosts": sorted(model_hosts)}
