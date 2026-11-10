#!/usr/bin/env python3
"""Audit an exported Figma node snapshot without claiming remote mutation."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import sys
from pathlib import Path
from typing import Any


def fail(message: str) -> None:
    raise ValueError(message)


def load_object(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        fail("snapshot must be a JSON object")
    return value


def hex_rgb(value: str) -> tuple[float, float, float]:
    text = value.strip().lstrip("#")
    if len(text) != 6:
        fail(f"unsupported color: {value}")
    return tuple(int(text[index:index + 2], 16) / 255 for index in (0, 2, 4))


def luminance(value: str) -> float:
    channels = []
    for channel in hex_rgb(value):
        channels.append(channel / 12.92 if channel <= 0.04045 else math.pow((channel + 0.055) / 1.055, 2.4))
    return 0.2126 * channels[0] + 0.7152 * channels[1] + 0.0722 * channels[2]


def contrast_ratio(first: str, second: str) -> float:
    bright, dark = sorted((luminance(first), luminance(second)), reverse=True)
    return round((bright + 0.05) / (dark + 0.05), 2)


def audit(snapshot: dict[str, Any]) -> dict[str, Any]:
    source_url = str(snapshot.get("source_url", "")).strip()
    file_key = str(snapshot.get("file_key", "")).strip()
    page = str(snapshot.get("page", "")).strip()
    target_node_id = str(snapshot.get("target_node_id", "")).strip()
    nodes = snapshot.get("nodes")
    variables = snapshot.get("variables")
    if not source_url.startswith("https://www.figma.com/") or not file_key or not page or not target_node_id:
        fail("source_url, file_key, page, and target_node_id are required")
    if not isinstance(nodes, list) or not nodes or not all(isinstance(item, dict) for item in nodes):
        fail("nodes must be a populated list")
    if not isinstance(variables, dict):
        fail("variables must be an object")

    ids = [str(node.get("id", "")).strip() for node in nodes]
    if any(not item for item in ids) or len(set(ids)) != len(ids):
        fail("node IDs must be present and unique")
    if target_node_id not in ids:
        fail("target node is absent from snapshot")

    spacing_tokens = {
        float(value) for name, value in variables.items()
        if str(name).startswith("spacing/") and isinstance(value, (int, float))
    }
    findings: list[dict[str, Any]] = []
    for node in nodes:
        node_id = str(node["id"])
        name = str(node.get("name", node_id))
        height = float(node.get("height", 0) or 0)
        item_spacing = node.get("item_spacing")
        if str(node.get("type", "")).upper() in {"INSTANCE", "COMPONENT"} and height and height < 44:
            findings.append({
                "severity": "high",
                "node_id": node_id,
                "node_name": name,
                "rule": "minimum-interactive-height",
                "evidence": f"height={height:g}px; minimum=44px",
                "impact": "The control is harder to target with touch or reduced motor precision.",
                "recommendation": "Increase the component minimum height to 44px and recheck all instances.",
                "confidence": 1.0,
            })
        states = {str(item).lower() for item in node.get("states", [])}
        if str(node.get("type", "")).upper() in {"INSTANCE", "COMPONENT"} and "focus" not in states:
            findings.append({
                "severity": "high",
                "node_id": node_id,
                "node_name": name,
                "rule": "focus-state",
                "evidence": "states=" + ",".join(sorted(states)),
                "impact": "Keyboard users cannot verify which control owns focus.",
                "recommendation": "Add a focus-visible variant without detaching existing instances.",
                "confidence": 0.98,
            })
        if isinstance(item_spacing, (int, float)) and spacing_tokens and float(item_spacing) not in spacing_tokens:
            findings.append({
                "severity": "medium",
                "node_id": node_id,
                "node_name": name,
                "rule": "spacing-variable",
                "evidence": f"item_spacing={float(item_spacing):g}px; available={sorted(spacing_tokens)}",
                "impact": "The frame does not follow the shared spacing rhythm.",
                "recommendation": "Map item spacing to the nearest approved variable after design review.",
                "confidence": 1.0,
            })

    color_checks = []
    accent = variables.get("color/accent")
    on_accent = variables.get("color/on-accent")
    if isinstance(accent, str) and isinstance(on_accent, str):
        ratio = contrast_ratio(accent, on_accent)
        color_checks.append({
            "foreground_variable": "color/on-accent",
            "background_variable": "color/accent",
            "ratio": ratio,
            "wcag_aa_normal_text": ratio >= 4.5,
        })

    canonical = json.dumps(snapshot, ensure_ascii=True, sort_keys=True, separators=(",", ":"))
    return {
        "schema_version": 1,
        "mode": "advisory",
        "applied": False,
        "connection": "offline_snapshot",
        "source": {
            "url": source_url,
            "file_key": file_key,
            "page": page,
            "target_node_id": target_node_id,
            "snapshot_sha256": hashlib.sha256(canonical.encode("utf-8")).hexdigest(),
        },
        "inventory": {
            "node_count": len(nodes),
            "component_count": sum(str(node.get("type", "")).upper() in {"COMPONENT", "INSTANCE"} for node in nodes),
            "variable_count": len(variables),
            "node_ids": ids,
        },
        "findings": findings,
        "color_checks": color_checks,
        "acceptance_criteria": [
            "Re-read the same node IDs through an authorized connector before any edit.",
            "Preserve Auto Layout, component relationships, instance overrides, and variable semantics.",
            "Capture an after snapshot only after the user approves a scoped edit.",
            "Do not report Applied without a connector mutation receipt and a successful re-read.",
        ],
        "mutation_receipt": None,
    }


def validate(path: Path) -> dict[str, Any]:
    result = load_object(path)
    if result.get("mode") != "advisory" or result.get("applied") is not False:
        fail("offline handoff must remain advisory and unapplied")
    source = result.get("source")
    inventory = result.get("inventory")
    findings = result.get("findings")
    if not isinstance(source, dict) or len(str(source.get("snapshot_sha256", ""))) != 64:
        fail("snapshot provenance is incomplete")
    if not isinstance(inventory, dict) or int(inventory.get("node_count", 0)) < 1:
        fail("node inventory is empty")
    node_ids = set(inventory.get("node_ids", []))
    if not isinstance(findings, list) or not findings:
        fail("audit has no findings")
    if any(str(item.get("node_id", "")) not in node_ids for item in findings if isinstance(item, dict)):
        fail("finding references an unknown node")
    if result.get("mutation_receipt") is not None:
        fail("offline audit cannot contain a mutation receipt")
    return {
        "validated": True,
        "mode": "advisory",
        "applied": False,
        "node_count": int(inventory["node_count"]),
        "finding_count": len(findings),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--validate-only", action="store_true")
    args = parser.parse_args()
    try:
        if args.validate_only:
            receipt = validate(args.output)
        else:
            if args.input is None:
                fail("--input is required when auditing a snapshot")
            result = audit(load_object(args.input))
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(json.dumps(result, ensure_ascii=True, indent=2, sort_keys=True) + "\n", encoding="utf-8")
            receipt = validate(args.output)
        print(json.dumps(receipt, ensure_ascii=True, sort_keys=True))
        return 0
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(json.dumps({"validated": False, "error": str(error)}, ensure_ascii=True), file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
