#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Summarize Rust diagnostic JSONL; never infer end-to-end or UI latency.

Usage: python3 scripts/report_measurement_events.py BUILD_SHA256 FILE [FILE ...]
Writes a sanitized report to stdout. Inputs are read only; no business DB access.
"""
import collections
import argparse
import hashlib
import json
import math
import pathlib
import re
import statistics
import sys


STAGES = frozenset(("provider_sampling", "http_headers", "adapter_first_text",
                    "adapter_first_reasoning", "host_tool", "host_compaction",
                    "store_commit", "store_committed_host_response",
                    "command_execution", "command_receipt_committed"))
OUTCOMES = frozenset(("succeeded", "failed", "cancelled", "rejected", "incomplete"))
COMMAND_KINDS = frozenset(("begin_turn", "active_submission", "idle_submission", "queued_turn", "task_turn", "automation_turn", "other"))


def natural(value):
    return type(value) is int and value >= 0


def decimal(value):
    return isinstance(value, str) and re.fullmatch(r"[0-9]{1,40}", value) is not None


def distribution(values):
    values = sorted(values)
    return {"count": len(values), "median_ms": statistics.median(values),
        "p95_ms": values[math.ceil(len(values) * .95) - 1], "max_ms": values[-1]}


def first_output_report(spans, observations, conflicted_spans=frozenset()):
    """Same-clock adapter onset only, with an explicit observed-attempt denominator.

    Parentage and source identity must both match. A missing end is censored;
    duplicate milestones are ambiguous, not an opportunity to pick the fastest.
    """
    stages = ("adapter_first_text", "adapter_first_reasoning")
    children = collections.defaultdict(list)
    rejected = collections.Counter()
    for row in observations:
        if row["stage"] not in stages:
            continue
        context = row.get("context")
        parent = context.get("parent_span") if isinstance(context, dict) else None
        key = (row["process_id"], row["clock_domain"], parent)
        if not natural(parent) or key not in spans or not any(
                r["stage"] == "provider_sampling" for r in spans[key]):
            rejected["missing_sampling_parent"] += 1
            continue
        children[(key, row["stage"])].append(row)
    result = {}
    for stage in stages:
        counts = collections.Counter()
        durations = collections.defaultdict(list)
        for key, rows in spans.items():
            if not any(r["stage"] == "provider_sampling" for r in rows):
                continue
            counts["observed_sampling_spans"] += 1
            begin = [r for r in rows if r["event"] == "begin"]
            end = [r for r in rows if r["event"] == "end"]
            if (key in conflicted_spans or len(begin) != 1 or len(end) > 1
                    or any(r["stage"] != "provider_sampling" for r in rows)
                    or not isinstance(begin[0].get("context"), dict)
                    or (end and (begin[0].get("context") != end[0].get("context")
                        or int(end[0]["monotonic_ns"]) < int(begin[0]["monotonic_ns"])) )):
                counts["invalid_sampling_spans"] += 1
                continue
            points = children.get((key, stage), [])
            if not points:
                counts["without_observed_output"] += 1
                continue
            origin = begin[0]
            if len(points) != 1:
                counts["invalid_output_observations"] += 1
                continue
            point = points[0]
            context = point["context"]
            if (any(context.get(field) != origin["context"].get(field)
                    for field in ("session", "effect", "request", "attempt"))
                    or int(point["monotonic_ns"]) < int(origin["monotonic_ns"])
                    or (end and int(point["monotonic_ns"]) > int(end[0]["monotonic_ns"]))):
                counts["invalid_output_observations"] += 1
                continue
            outcome = end[0]["outcome"] if end else "censored"
            counts["with_observed_output"] += 1
            durations[outcome].append((int(point["monotonic_ns"]) - int(origin["monotonic_ns"])) / 1_000_000)
        result[stage] = {**counts, "elapsed_by_sampling_outcome": {
            outcome: distribution(values) for outcome, values in sorted(durations.items())}}
    return {"stages": result, "rejected_points": dict(rejected),
        "denominator": "Observed provider_sampling spans, not all user submissions or HTTP requests.",
        "endpoint": "Adapter callback before downstream admission; not network first byte or UI rendering."}


def command_receipt_report(observations, experiment=None, canonical=None, conflicted_commands=frozenset()):
    """Reconcile lossy points against an explicit, independently exported receipt snapshot.

    A committed receipt with a rejected outcome is NOT an accepted business command.
    No inference is made from absence of sampling or from a missing diagnostic point.
    """
    digest = lambda value: isinstance(value, str) and re.fullmatch(r"[0-9a-f]{64}", value) is not None
    observed = collections.defaultdict(list)
    invalid = 0
    for row in observations:
        if row["stage"] != "command_receipt_committed":
            continue
        context = row.get("context")
        if (not isinstance(context, dict) or not digest(context.get("command"))
                or row.get("outcome") not in ("succeeded", "rejected", "incomplete")):
            invalid += 1
            continue
        observed[context["command"]].append(row)
    ambiguous = set(conflicted_commands)
    for command, rows in observed.items():
        if len(rows) != 1:
            ambiguous.add(command)
    result = {"observed_unique_receipts": len(set(observed) - ambiguous),
        "ambiguous_commands": len(ambiguous), "invalid_points": invalid,
        "denominator": "No experiment supplied; observed receipts are not the submission denominator."}
    if experiment is None:
        return result
    samples = experiment.get("samples")
    receipts = canonical.get("receipts") if isinstance(canonical, dict) else None
    if not isinstance(samples, list) or not isinstance(receipts, list):
        raise ValueError("experiment samples and canonical receipts must be lists")
    sample_map, receipt_map = {}, {}
    for sample in samples:
        if (not isinstance(sample, dict) or not digest(sample.get("command"))
                or sample["command"] in sample_map or sample.get("result") not in
                ("succeeded", "failed", "cancelled", "timeout", "pending")):
            raise ValueError("invalid or duplicate experiment sample identity/result")
        sample_map[sample["command"]] = sample
    for receipt in receipts:
        if (not isinstance(receipt, dict) or not digest(receipt.get("command"))
                or receipt["command"] in receipt_map or receipt.get("status") not in
                ("committed", "rejected", "conflict", "recovery_required", "absent", "unknown")
                or any(receipt.get(field) is not None and not digest(receipt[field]) for field in ("session", "run"))):
            raise ValueError("invalid or duplicate canonical receipt identity/status")
        receipt_map[receipt["command"]] = receipt
    counts, outcomes, reconciliation = collections.Counter(), collections.Counter(), collections.Counter()
    expected = {"committed": "succeeded", "rejected": "rejected", "conflict": "rejected",
        "recovery_required": "incomplete"}
    for command, sample in sample_map.items():
        outcomes[sample["result"]] += 1
        receipt = receipt_map.get(command)
        status = receipt["status"] if receipt else "missing_snapshot_entry"
        counts[status] += 1
        if command in ambiguous:
            reconciliation["ambiguous_observation"] += 1
        elif command not in observed:
            reconciliation["no_observed_receipt"] += 1
        elif status not in expected:
            reconciliation["receipt_not_confirmed"] += 1
        else:
            point = observed[command][0]
            matches = point["outcome"] == expected[status] and all(
                receipt.get(field) is None or receipt[field] == point["context"].get(field)
                for field in ("session", "run"))
            reconciliation["matched" if matches else "receipt_mismatch"] += 1
    return {**result, "denominator": "All experiment samples; canonical receipt snapshot confirms admission, not diagnostic events.",
        "all_samples": len(samples), "committed_command_samples": counts["committed"],
        "canonical_receipt_statuses": dict(counts), "sample_results": dict(outcomes),
        "reconciliation": dict(reconciliation),
        "observed_commands_outside_experiment": len(set(observed) - set(sample_map)),
        "snapshot_commands_outside_experiment": len(set(receipt_map) - set(sample_map))}


def command_output_report(spans, points, conflicted_spans, experiment, canonical):
    """Canonical tuple joins only; never assign an attempt by session/time proximity."""
    if experiment is None or canonical is None:
        return {"available": False, "reason": "experiment_and_canonical_snapshot_required"}
    receipts = {r["command"]: r for r in canonical.get("receipts", [])}
    bindings = collections.defaultdict(list)
    expected_run_attempts = collections.defaultdict(set)
    observed_run_attempts = collections.defaultdict(set)
    output_run_attempts = collections.defaultdict(set)
    attempt_ids, retry_ids, compaction_ids, maintenance_ids = set(), set(), set(), set()
    for row in canonical.get("attempts", []):
        if not isinstance(row, dict):
            raise ValueError("invalid canonical attempt binding")
        for field in ("attempt", "run", "session", "effect"):
            if not isinstance(row.get(field), str) or re.fullmatch(r"[0-9a-f]{64}", row[field]) is None:
                raise ValueError("invalid canonical attempt identity")
        attempt_ids.add(row["attempt"])
        if row.get("purpose") == "conversation" and row.get("run_kind") == "conversation":
            expected_run_attempts[row["run"]].add(row["attempt"])
        if row.get("predecessor") is not None:
            retry_ids.add(row["attempt"])
        if row.get("purpose") == "compaction":
            compaction_ids.add(row["attempt"])
        if row.get("run_kind") == "maintenance":
            maintenance_ids.add(row["attempt"])
        key = tuple(row.get(field) for field in ("session", "effect", "request", "attempt"))
        bindings[key].append(row)
    run_points = collections.defaultdict(list)
    joins = collections.Counter()
    for key, rows in spans.items():
        if not any(r["stage"] == "provider_sampling" for r in rows):
            continue
        begins = [r for r in rows if r["event"] == "begin"]
        ends = [r for r in rows if r["event"] == "end"]
        if (key in conflicted_spans or len(begins) != 1 or len(ends) > 1
                or any(r["stage"] != "provider_sampling" for r in rows)):
            joins["invalid_sampling_span"] += 1
            continue
        begin = begins[0]
        context = begin.get("context")
        if not isinstance(context, dict):
            joins["invalid_sampling_span"] += 1
            continue
        identity = tuple(context.get(f) for f in ("session", "effect", "request", "attempt"))
        matches = bindings.get(identity, []) if all(isinstance(v, str) for v in identity) else []
        if len(matches) != 1:
            joins["missing_or_ambiguous_canonical_binding"] += 1
            continue
        binding = matches[0]
        if ends and (ends[0].get("context") != context or int(ends[0]["monotonic_ns"]) < int(begin["monotonic_ns"])):
            joins["invalid_sampling_span"] += 1
            continue
        joins["matched_sampling_spans"] += 1
        if binding.get("purpose") != "conversation" or binding.get("run_kind") != "conversation":
            joins["non_conversation_sampling_spans"] += 1
            continue
        observed_run_attempts[binding["run"]].add(binding["attempt"])
        for stage in ("adapter_first_text", "adapter_first_reasoning"):
            candidates = [p for p in points if p["stage"] == stage and isinstance(p.get("context"), dict) and
                (p["process_id"], p["clock_domain"], p["context"].get("parent_span")) == key]
            if len(candidates) != 1:
                continue
            point = candidates[0]
            if (any(point["context"].get(f) != context.get(f) for f in ("session", "effect", "request", "attempt"))
                    or int(point["monotonic_ns"]) < int(begin["monotonic_ns"])
                    or ends and int(point["monotonic_ns"]) > int(ends[0]["monotonic_ns"])):
                continue
            run_points[(binding["run"], stage)].append(point)
            output_run_attempts[(binding["run"], stage)].add(binding["attempt"])
    run_owners = collections.Counter(r.get("run") for r in receipts.values()
        if r.get("run_binding") == "opened_run" and r.get("run"))
    results = {}
    for stage in ("adapter_first_text", "adapter_first_reasoning"):
        counts, durations = collections.Counter(), collections.defaultdict(list)
        for sample in experiment["samples"]:
            counts["all_samples"] += 1
            receipt = receipts.get(sample["command"], {})
            run = receipt.get("run")
            if receipt.get("status") != "committed":
                counts["not_canonically_committed"] += 1
                continue
            if not run:
                counts["missing_run_binding"] += 1
                continue
            if receipt.get("run_binding") != "opened_run" or run_owners[run] != 1:
                counts["shared_or_existing_run_not_attributed"] += 1
                continue
            output = run_points.get((run, stage), [])
            expected_attempts = expected_run_attempts[run]
            if expected_attempts - observed_run_attempts[run]:
                counts["samples_with_unobserved_canonical_attempts"] += 1
            if expected_attempts - output_run_attempts[(run, stage)]:
                # A missing first-output point is not proof that the attempt
                # produced nothing: its diagnostic event may have been lost.
                counts["samples_without_output_proof_for_every_attempt"] += 1
            if not output:
                counts["without_observed_output"] += 1
                continue
            admissions = [p for p in points if p["stage"] == "command_receipt_committed"
                and isinstance(p.get("context"), dict) and p["context"].get("command") == sample["command"]]
            if len(admissions) != 1:
                counts["missing_or_ambiguous_command_origin"] += 1
                continue
            admission = admissions[0]
            if admission.get("outcome") != "succeeded" or any(admission["context"].get(f) != receipt.get(f)
                    for f in ("run", "session")):
                counts["command_receipt_identity_mismatch"] += 1
                continue
            parent = (admission["process_id"], admission["clock_domain"], admission["context"].get("parent_span"))
            origin_rows = spans.get(parent, [])
            origins = [r for r in origin_rows if r["event"] == "begin"]
            terminals = [r for r in origin_rows if r["event"] == "end"]
            if (parent in conflicted_spans or len(origins) != 1
                    or len(terminals) > 1
                    or any(r["stage"] != "command_execution" for r in origin_rows)
                    or origins[0].get("context", {}).get("command") != sample["command"]):
                counts["missing_or_ambiguous_command_origin"] += 1
                continue
            origin = origins[0]
            if (int(admission["monotonic_ns"]) < int(origin["monotonic_ns"])
                    or terminals and (terminals[0].get("context") != origin.get("context")
                        or int(terminals[0]["monotonic_ns"]) < int(admission["monotonic_ns"]))):
                counts["invalid_command_admission_order"] += 1
                continue
            if any((p["process_id"], p["clock_domain"]) != parent[:2] for p in output):
                counts["cross_clock_output_no_latency"] += 1
                continue
            first = min(int(p["monotonic_ns"]) for p in output)
            if first < int(admission["monotonic_ns"]):
                counts["output_precedes_admission"] += 1
                continue
            counts["with_same_clock_observed_onset"] += 1
            durations[sample["result"]].append((first - int(origin["monotonic_ns"])) / 1_000_000)
        results[stage] = {**counts, "observed_onset_by_sample_result": {k: distribution(v) for k, v in durations.items()},
            "canonical_conversation_attempts": sum(len(v) for v in expected_run_attempts.values()),
            "sampling_observed_attempts": sum(len(v) for v in observed_run_attempts.values()),
            "output_observed_attempts": sum(len(v) for (run, kind), v in output_run_attempts.items() if kind == stage)}
    return {"available": True, "stages": results, "sampling_joins": dict(joins),
        "canonical_attempts": len(attempt_ids), "retry_attempts": len(retry_ids),
        "compaction_attempts": len(compaction_ids), "maintenance_attempts": len(maintenance_ids),
        "endpoint": "Application call begin to earliest OBSERVED adapter output in one clock domain: a lossy proxy, never proof of actual Run first output, UI submission, network first byte or frame."}


def summarize(records, build_digest, experiment=None, canonical=None):
    if re.fullmatch(r"[0-9a-fA-F]{64}", build_digest) is None:
        raise ValueError("a 64-digit build digest is required")
    spans = collections.defaultdict(list)
    seen = collections.defaultdict(list)
    conflicted_spans = set()
    conflicted_commands = set()
    dropped = {}
    points = collections.Counter()
    point_observations = []
    rejected = collections.Counter()
    for row in records:
        if not isinstance(row, dict):
            rejected["invalid_schema"] += 1
            continue
        if row.get("build_digest") != build_digest:
            rejected["different_or_missing_build"] += 1
            continue
        domain = row.get("clock_domain")
        stage, event = row.get("stage"), row.get("event")
        if (type(row.get("schema_version")) is not int or row["schema_version"] != 1
                or not isinstance(domain, str) or not domain
                or not all(natural(row.get(k)) for k in ("process_id", "observation_id", "span_id", "dropped_total"))
                or not isinstance(stage, str) or stage not in STAGES
                or not isinstance(event, str) or event not in ("begin", "end", "point")
                or not decimal(row.get("monotonic_ns"))
                or (event == "end" and (not isinstance(row.get("outcome"), str)
                    or row["outcome"] not in OUTCOMES or not decimal(row.get("elapsed_ns"))))):
            rejected["invalid_schema"] += 1
            continue
        clock = (row["process_id"], domain)
        if stage in ("command_execution", "command_receipt_committed"):
            context = row.get("context")
            if (not isinstance(context, dict) or not isinstance(context.get("command"), str)
                    or re.fullmatch(r"[0-9a-f]{64}", context["command"]) is None
                    or not isinstance(context.get("command_kind"), str)
                    or context["command_kind"] not in COMMAND_KINDS):
                rejected["invalid_command_identity"] += 1
                continue
        identity = (*clock, row["observation_id"])
        if row in seen[identity]:
            rejected["duplicate_observation"] += 1
            continue
        if seen[identity]:
            rejected["conflicting_observation"] += 1
            for candidate in [*seen[identity], row]:
                conflicted_spans.add((*clock, candidate["span_id"]))
                context = candidate.get("context")
                if candidate["stage"] == "command_receipt_committed" and isinstance(context, dict):
                    command = context.get("command")
                    if isinstance(command, str):
                        conflicted_commands.add(command)
                parent = context.get("parent_span") if isinstance(context, dict) else None
                if candidate["event"] == "point" and natural(parent):
                    conflicted_spans.add((*clock, parent))
        seen[identity].append(row)
        dropped[clock] = max(dropped.get(clock, 0), row["dropped_total"])

    # Conflicts must invalidate earlier observations too. Never select a winner
    # by file order, even when that would produce a plausible complete span.
    for variants in seen.values():
        for row in variants:
            clock = (row["process_id"], row["clock_domain"])
            event, stage = row["event"], row["stage"]
            if event == "point" and len(variants) != 1:
                continue
            if event == "point":
                points[stage] += 1
                point_observations.append(row)
            else:
                spans[(*clock, row["span_id"])].append(row)

    stages = collections.defaultdict(lambda: {"observed_spans": 0, "unpaired": 0,
        "invalid_pairs": 0, "outcomes": collections.Counter(), "durations": collections.defaultdict(list)})
    for key, rows in spans.items():
        named_stages = sorted({row["stage"] for row in rows})
        if len(named_stages) != 1:
            # A corrupted span naming multiple stages has no authoritative
            # owner. Expose the invalid association under every named stage,
            # never attribute it to whichever source file was opened first.
            for stage in named_stages:
                stages[stage]["observed_spans"] += 1
                stages[stage]["invalid_pairs"] += 1
            continue
        bucket = stages[named_stages[0]]
        bucket["observed_spans"] += 1
        begin = [r for r in rows if r["event"] == "begin"]
        end = [r for r in rows if r["event"] == "end"]
        if key in conflicted_spans:
            bucket["invalid_pairs"] += 1
            continue
        if not begin or not end:
            bucket["unpaired"] += 1
            continue
        if (len(begin) != 1 or len(end) != 1 or begin[0]["stage"] != end[0]["stage"]
                or begin[0].get("context") != end[0].get("context")
                or int(end[0]["monotonic_ns"]) < int(begin[0]["monotonic_ns"])):
            bucket["invalid_pairs"] += 1
            continue
        # Recorder elapsed_ns uses its span's Instant, not emit timestamps.
        # Never subtract timestamps across process/domain or sum overlapping spans.
        outcome = end[0]["outcome"]
        bucket["outcomes"][outcome] += 1
        bucket["durations"][outcome].append(int(end[0]["elapsed_ns"]) / 1_000_000)
    output = {}
    for stage, bucket in sorted(stages.items()):
        distributions = {}
        for outcome, durations in sorted(bucket.pop("durations").items()):
            distributions[outcome] = distribution(durations)
        output[stage] = {**bucket, "elapsed_by_outcome": distributions}
    return {"schema_version": 1, "build_digest": build_digest, "stages": output,
        "point_counts": dict(points), "rejected_records": dict(rejected),
        "adapter_first_output": first_output_report(spans, point_observations, conflicted_spans),
        "command_receipts": command_receipt_report(point_observations, experiment, canonical, conflicted_commands),
        "command_output": command_output_report(spans, point_observations, conflicted_spans, experiment, canonical),
        "observed_dropped_lower_bound": sum(dropped.values()),
        "limitations": ["Admission denominator requires an experiment and canonical receipt snapshot; diagnostics alone are incomplete.",
            "Committed command receipts do not prove a new Run, task completion, or a model request.",
            "Drop counters are lower bounds; process-exit tails may be missing.",
            "Point counts are not latency or physical-request counts.",
            "Incomplete spans are censored, not successful latency samples.",
            "No UI-frame, cross-process, end-to-end, cache-rate or budget conclusion."]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("build_digest")
    parser.add_argument("files", nargs="+")
    parser.add_argument("--experiment")
    parser.add_argument("--receipts")
    args = parser.parse_args()
    if bool(args.experiment) != bool(args.receipts):
        parser.error("--experiment and --receipts must be supplied together")
    experiment = canonical = None
    if args.experiment:
        experiment = json.loads(pathlib.Path(args.experiment).read_bytes())
        raw = pathlib.Path(args.receipts).read_bytes()
        canonical = json.loads(raw)
        if (experiment.get("schema_version") != 1 or canonical.get("schema_version") != 1
                or experiment.get("build_digest") != args.build_digest
                or canonical.get("build_digest") != args.build_digest
                or experiment.get("canonical_receipts_sha256") != hashlib.sha256(raw).hexdigest()):
            parser.error("experiment/receipt snapshot schema, build or digest mismatch")
    manifests = []
    def records():
        for name in args.files:
            digest = hashlib.sha256()
            with pathlib.Path(name).open("rb") as source:
                for line in source:
                    digest.update(line)
                    try:
                        yield json.loads(line)
                    except (ValueError, UnicodeError):
                        yield None
            # Do not disclose absolute source paths in exported diagnostics.
            manifests.append({"source_index": len(manifests), "sha256": digest.hexdigest()})
    report = summarize(records(), args.build_digest, experiment, canonical)
    report["sources"] = manifests
    print(json.dumps(report, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
