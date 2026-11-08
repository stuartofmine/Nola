#!/usr/bin/env python3
"""Audited E2E for messaging ingress, delivery, persistence, and Host management."""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def run_json(script: str, timeout: int) -> dict:
    completed = subprocess.run(
        [sys.executable, str(ROOT / "scripts" / script)],
        cwd=ROOT,
        text=True,
        capture_output=True,
        timeout=timeout,
    )
    if completed.returncode != 0:
        raise RuntimeError(
            f"{script} failed ({completed.returncode})\n"
            f"stdout:\n{completed.stdout[-6000:]}\n"
            f"stderr:\n{completed.stderr[-6000:]}"
        )
    lines = [line for line in completed.stdout.splitlines() if line.strip().startswith("{")]
    if not lines:
        raise RuntimeError(f"{script} produced no machine-readable result")
    return json.loads(lines[-1])


def main() -> int:
    sys.path.insert(0, str(ROOT / "scripts"))
    from real_e2e_evidence import record_step

    ingress = run_json("test_messaging_webhooks_e2e.py", 180)
    delivery = run_json("test_messaging_delivery_e2e.py", 180)
    management = run_json("test_messaging_channel_web_proxy_e2e.py", 90)
    protocols = set(ingress.get("protocols", []))
    delivered_protocols = set(delivery.get("protocols", []))
    expected = {"telegram", "whatsapp", "wechat_official", "we_com"}
    assertions = {
        "all_official_webhooks_accepted_valid_signatures": protocols == expected,
        "invalid_signatures_rejected": ingress.get("invalid_signatures_rejected") is True,
        "duplicate_provider_events_deduplicated": ingress.get("deduplication") is True,
        "inbox_projection_persisted": ingress.get("inbox_count") == 4,
        "provider_replies_delivered_and_persisted": (
            delivered_protocols == expected
            and delivery.get("delivered") == 4
            and delivery.get("durable_sent") == 4
        ),
        "host_channel_management_token_not_leaked": (
            management.get("proxied") == ["GET", "PUT"]
            and management.get("token_leaked") is False
        ),
    }
    if not all(assertions.values()):
        raise RuntimeError(f"messaging full-stack assertions failed: {assertions}")

    evidence_root = Path(
        os.environ.get("NOLA_E2E_ARTIFACT_ROOT")
        or tempfile.mkdtemp(prefix="nola-messaging-full-stack-")
    ) / "messaging-full-stack"
    evidence_root.mkdir(parents=True, exist_ok=True)
    events = evidence_root / "events.ndjson"
    store = evidence_root / "persistence-store.json"
    audit = evidence_root / "persistence-audit.json"
    rows = [
        {"type": "channel.ingress", "protocol": protocol, "status": "persisted"}
        for protocol in sorted(expected)
    ] + [
        {"type": "channel.delivery", "protocol": protocol, "status": "sent"}
        for protocol in sorted(expected)
    ] + [
        {"type": "host.channel_management", "operations": ["GET", "PUT"], "token_leaked": False},
        {"type": "done", "sequence": 10, "status": "completed"},
    ]
    events.write_text("".join(json.dumps(row, ensure_ascii=False) + "\n" for row in rows))
    store.write_text(json.dumps({
        "ingress": ingress,
        "delivery": delivery,
        "management": management,
    }, ensure_ascii=False, indent=2))
    audit.write_text(json.dumps({
        "passed": True,
        "assertions": assertions,
    }, ensure_ascii=False, indent=2))
    observations = [
        {"id": "messaging-events", "kind": "event_stream", "path": events},
        {"id": "messaging-store", "kind": "persistence_store", "path": store},
        {"id": "messaging-audit", "kind": "persistence_audit", "path": audit},
    ]
    modules = ["messaging", "webhooks", "delivery", "persistence", "security", "web-host"]
    boundaries = [
        "telegram-webhook", "whatsapp-webhook", "wechat-official-webhook",
        "wecom-webhook", "provider-delivery", "d1", "web-host-channel-proxy",
    ]
    evidence_map = {key: ["messaging-events", "messaging-store", "messaging-audit"] for key in assertions}
    for step_id, platform in [
        ("telegram-ingress", "backend"),
        ("whatsapp-ingress", "backend"),
        ("wechat-official-ingress", "backend"),
        ("wecom-ingress", "backend"),
        ("provider-delivery-and-d1", "backend"),
        ("host-channel-management", "web"),
    ]:
        record_step(
            step_id,
            platforms=[platform],
            modules=modules,
            boundaries=boundaries,
            assertions=assertions,
            assertion_evidence=evidence_map,
            observations=observations,
            metrics={"ingress": 4, "delivered": 4, "providerRequests": delivery["provider_requests"]},
        )
    print(json.dumps({
        "ok": True,
        "protocols": sorted(expected),
        "ingress": 4,
        "delivered": 4,
        "durable_sent": 4,
        "host_management": True,
    }))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
