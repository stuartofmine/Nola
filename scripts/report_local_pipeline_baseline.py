#!/usr/bin/env python3
"""Persist sanitized baseline artifacts and produce nearest-rank latency summaries.

Usage: python3 scripts/report_local_pipeline_baseline.py RAW_JSON TEST_BINARY OUTPUT_DIR
Generated JSON contains fixture measurements only; never pass production logs.
"""
import hashlib
import json
import math
import os
import pathlib
import platform
import statistics
import subprocess
import sys
from datetime import datetime, timezone


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def command(*args):
    return subprocess.check_output(args, text=True).strip()


def main():
    raw_path, binary, destination = map(pathlib.Path, sys.argv[1:])
    root = pathlib.Path(__file__).resolve().parents[1]
    report = json.loads(raw_path.read_text())
    assert report["scenario"] == "local-application-sqlite-v1"
    destination.mkdir(parents=True, exist_ok=False)
    (destination / "samples.json").write_text(json.dumps(report, indent=2) + "\n")
    sources = {}
    for folder in ("core", "apps/cli"):
        for directory, subdirectories, files in os.walk(root / folder):
            subdirectories[:] = [name for name in subdirectories if name not in ("target", ".git")]
            for name in files:
                path = pathlib.Path(directory) / name
                if path.suffix == ".rs" or path.name == "Cargo.toml":
                    sources[str(path.relative_to(root))] = sha(path)
    sources["Cargo.lock"] = sha(root / "Cargo.lock")
    (destination / "source-digests.json").write_text(json.dumps(sources, sort_keys=True, indent=2) + "\n")
    manifest = {"captured_at_utc": datetime.now(timezone.utc).isoformat(),
        "machine_model": command("sysctl", "-n", "hw.model"),
        "memory_bytes": int(command("sysctl", "-n", "hw.memsize")),
        "logical_cpus": int(command("sysctl", "-n", "hw.ncpu")),
        "os": command("sw_vers"), "architecture": platform.machine(),
        "rustc": command("rustc", "--version"), "test_binary_sha256": sha(binary),
        "samples_sha256": sha(destination / "samples.json"),
        "source_manifest_sha256": sha(destination / "source-digests.json"),
        "source_identity_caveat": "Dirty shared worktree snapshot captured after run; binary digest is authoritative for executed artifact.",
        "build_command": "nola_cargo test -p nola_cli --test local_pipeline_baseline -- --ignored --nocapture",
        "runtime": {"NOLA_HOME": "isolated temporary directory", "RUST_MIN_STACK": 16777216},
        "missing": ["provider", "UI frame", "IPC ACK", "process cold start", "OS page-cache cold", "power-loss durability"]}
    (destination / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    stages = ["admission_receipt", "claim_receipt", "execution_started_receipt", "attempt_started_receipt",
              "delta_1024b_receipt", "reopen_connection", "recovered_journal_head", "recovered_session_projection"]
    summary = []
    for condition in ("fresh_database", "reused_database"):
        for stage in stages + (["fresh_database_open"] if condition == "fresh_database" else []):
            rows = [r for r in report["samples"] if r["condition"] == condition and r["stage"] == stage]
            values = sorted(int(r["elapsed_ns"]) / 1_000_000 for r in rows if r["outcome"] == "succeeded")
            summary.append({"condition": condition, "stage": stage, "expected": 30,
                "observed": len(rows), "failed": sum(r["outcome"] != "succeeded" for r in rows),
                "missing": 30-len(rows), "median_ms": statistics.median(values) if values else None,
                "p95_ms": values[math.ceil(len(values)*.95)-1] if values else None,
                "max_ms": max(values) if values else None})
    (destination / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    segments = []
    receipt_segment_count = 2 if report.get("commit_path") == "database_only_ack" else 3
    for condition in ("fresh_database", "reused_database"):
        for stage in stages[:5]:
            rows = [r for r in report["samples"] if r["condition"] == condition and r["stage"] == stage]
            for segment in ("disclosure_flush", "sql_commit", "post_commit_archive", "unattributed_remainder"):
                values = []
                outcomes = {}
                dropped = 0
                for row in rows:
                    diagnostic = row.get("commit_diagnostics", {})
                    observed = diagnostic.get("observations", [])
                    dropped += diagnostic.get("dropped", 0)
                    if segment == "unattributed_remainder":
                        if len(observed) == receipt_segment_count and not diagnostic.get("dropped", 0):
                            value = int(row["elapsed_ns"]) - sum(int(o["elapsed_ns"]) for o in observed)
                            assert value >= 0, "non-overlapping commit stages exceeded parent span"
                            values.append(value / 1_000_000)
                        continue
                    for observation in observed:
                        if observation["stage"] != segment:
                            continue
                        outcome = observation["outcome"]
                        outcomes[outcome] = outcomes.get(outcome, 0) + 1
                        # Preserve all outcomes separately; latency summarizes observations, not success alone.
                        values.append(int(observation["elapsed_ns"]) / 1_000_000)
                values.sort()
                segments.append({"condition": condition, "parent_stage": stage, "segment": segment,
                    "observed": len(values), "expected": 0 if segment == "post_commit_archive" and receipt_segment_count == 2 else 30,
                    "outcomes": outcomes, "dropped": dropped,
                    "median_ms": statistics.median(values) if values else None,
                    "p95_ms": values[math.ceil(len(values)*.95)-1] if values else None,
                    "max_ms": max(values) if values else None})
    (destination / "segments.json").write_text(json.dumps(segments, indent=2) + "\n")
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
