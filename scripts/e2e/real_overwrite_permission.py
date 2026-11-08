#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Real-provider profile comparison against owned, temporary file operations.

No Cargo, browser, messaging, user file or source-configuration mutations.
The model selects an actual product tool; file bytes and canonical state are
independent oracles, not a claim inferred from the assistant's final answer.
"""
from __future__ import annotations

import argparse
import concurrent.futures
import hashlib
import json
import os
from pathlib import Path
import signal
import sqlite3
import subprocess
import sys
import tempfile
import time
import uuid

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
from real_e2e_events import load_events, summarize
from e2e.model_execution_fixture import fixture, runtime_capabilities


def audit_database(path: Path) -> dict:
    with sqlite3.connect(path.resolve().as_uri() + "?mode=ro", uri=True) as connection:
        connection.execute("BEGIN")
        runs = [{"id": row[0], "status": row[1]} for row in connection.execute("SELECT id,status FROM runs")]
        permissions = [{"status": row[0], "decision": row[1]} for row in connection.execute(
            "SELECT status,decision FROM permission_requests")]
        checkpoints = [json.loads(row[0]) for row in connection.execute("SELECT checkpoint_json FROM core_checkpoint_bodies")]
        pending = sum(len(checkpoint.get("state", {}).get("execution", {}).get("permissions", {}).get("pending", []))
            for checkpoint in checkpoints)
        invocation_profiles = []
        for (raw,) in connection.execute("SELECT config_json FROM run_invocation_payloads"):
            rules = json.loads(raw).get("permission_rules", {}).get("rules", [])
            allow_all = any(rule.get("source") == "invocation_policy" and rule.get("capability_pattern") == "*"
                and rule.get("resource_pattern") == "*" and rule.get("decision") == "allow" for rule in rules)
            invocation_profiles.append("allow_all" if allow_all else "cautious")
        # This is a replay base, not necessarily the current state. The cold
        # Application projection below folds committed deltas authoritatively.
        return {"runs": runs, "permission_rows": permissions, "base_checkpoint_pending_permissions": pending,
            "invocation_profiles": invocation_profiles}


def audit_cache(path: Path, projection: dict) -> dict:
    """Compare cold UI accounting with persisted physical Attempt receipts."""
    with sqlite3.connect(path.resolve().as_uri() + "?mode=ro", uri=True) as connection:
        rows = connection.execute("SELECT a.id,a.record_json FROM run_attempts a "
            "JOIN runs r ON r.id=a.run_id WHERE r.conversation_id=?", (projection.get("id"),)).fetchall()
        delivery_bindings = set(connection.execute(
            "SELECT d.attempt_id,d.request_id FROM run_attempt_deliveries d "
            "JOIN run_attempts a ON a.id=d.attempt_id "
            "JOIN runs r ON r.id=a.run_id WHERE r.conversation_id=?", (projection.get("id"),)))
    # Completed outbox rows and payload bodies are intentionally prunable,
    # including the last completed
    # model request while a tool awaits approval. Identity uses its durable
    # physical-delivery ledger, never payload retention or request-ID spelling.
    attempts = {attempt: json.loads(record) for attempt, record in rows}
    usage = projection.get("context_usage") or {}
    source = usage.get("prompt_cache_measurement") or {}
    bound = attempts.get(source.get("attempt_id"))
    bound_usage = (bound.get("usage") or {}) if bound else {}
    diagnostic = usage.get("prompt_cache_diagnostic")
    attempt_cache = bound_usage.get("cache_usage")
    recorded = [record.get("usage") for record in attempts.values() if record.get("usage") is not None]
    comparable = [value["cache_usage"] for value in recorded if isinstance(value.get("cache_usage"), dict)
        and isinstance(value["cache_usage"].get("input_tokens"), int)
        and isinstance(value["cache_usage"].get("read_tokens"), int)]
    inputs = sum(value["input_tokens"] for value in comparable)
    reads = sum(value["read_tokens"] for value in comparable)
    totals = usage.get("conversation_cache_usage") or {}
    assertions = {
        "diagnostic_bound_to_real_attempt": bool(bound)
            and (source.get("attempt_id"), source.get("request_id")) in delivery_bindings,
        "cold_diagnostic_matches_attempt_usage": isinstance(diagnostic, dict) and isinstance(attempt_cache, dict)
            and diagnostic.get("cacheUsage") == attempt_cache
            and diagnostic.get("cacheReadInputTokens") == bound_usage.get("cached_input_tokens")
            and diagnostic.get("cacheWriteInputTokens") == bound_usage.get("cache_write_input_tokens"),
        "cumulative_input_matches_physical_receipts": bool(comparable) and totals.get("input_tokens") == inputs,
        "cumulative_cache_read_matches_physical_receipts": bool(comparable) and totals.get("read_tokens") == reads,
        "cumulative_ratio_is_weighted": inputs > 0 and totals.get("hit_rate_bps") == reads * 10000 // inputs,
        "reported_attempt_coverage_matches": totals.get("reported_requests") == len(comparable),
    }
    return {"passed": all(assertions.values()), "assertions": assertions,
        "physical_reported_attempts": len(comparable), "input_tokens": inputs, "read_tokens": reads,
        "cold_hit_rate_bps": totals.get("hit_rate_bps")}


def audit_marker_visibility(summary: dict, marker: str, session_id: str) -> dict[str, bool]:
    """Use committed current journal bodies, not transport pointers or tool success."""
    parts = summary.get("journalParts")
    available = isinstance(parts, list) and bool(parts)
    owned = available and all(isinstance(part, dict)
        and isinstance(part.get("head"), dict)
        and part["head"].get("identity", {}).get("conversation_id") == session_id
        and isinstance(part.get("bodies"), dict)
        and all(isinstance(text, str) for text in part["bodies"].values()) for part in parts)
    if not owned:
        return {"canonical_journal_available": available, "journal_owner_matches": False,
            "summary_matches_canonical_answer": False, "canonical_answer_contains_marker": False,
            "visible_journal_contains_marker": False}
    final_text = "\n".join(part["bodies"]["markdown"] for part in parts
        if part["head"]["metadata"]["kind"] == "text"
        and part["head"]["metadata"]["data"].get("is_final") is True
        and "markdown" in part["bodies"]).strip().strip("`")
    visible = any(marker in text for part in parts for text in part["bodies"].values())
    # Tool arguments and other displayed metadata are also observable surfaces,
    # but cannot substitute for a final assistant answer on the allow_all side.
    visible = visible or any(marker in json.dumps(part["head"]["metadata"], ensure_ascii=False) for part in parts)
    return {"canonical_journal_available": True, "journal_owner_matches": True,
        "summary_matches_canonical_answer": summary.get("assistant") == final_text,
        "canonical_answer_contains_marker": marker in final_text,
        "visible_journal_contains_marker": visible}


def stop_owned_process(process: subprocess.Popen) -> None:
    if process.poll() is None:
        os.killpg(process.pid, signal.SIGTERM)
        try:
            process.wait(timeout=8)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait(timeout=8)


def isolated_environment(case: Path, profile: str) -> dict[str, str]:
    if profile not in {"cautious", "allow_all"}:
        raise ValueError("unsupported permission profile")
    environment = {key: value for key, value in os.environ.items()
        if not key.startswith(("NOLA_", "OPENAI_", "ANTHROPIC_", "MODEL_"))
        and key not in {"nola_backend_env", "DATABASE_URL", "DATABASE_PATH", "DB_PATH"}}
    paths = {"NOLA_HOME": case / "home", "NOLA_MAC_APP_SUPPORT": case / "app-support",
        "NOLA_EXTERNAL_EXECUTION_HOME": case / "execution-home"}
    for path in paths.values():
        path.mkdir(parents=True, exist_ok=True)
    environment.update({key: str(value) for key, value in paths.items()})
    environment.update(NOLA_DB_PATH=str(case / "nola.db"), NOLA_PERMISSION_PROFILE=profile,
        NOLA_CLI_INTERACTION_MODE="never", NOLA_SKIP_ONBOARDING="1")
    return environment


def scrub_case_credentials(case: Path) -> None:
    """Remove only this disposable case's credentials, retaining all evidence."""
    config = case / "home/.nola/config.json"
    if config.is_symlink():
        raise RuntimeError("fixture configuration became a symlink")
    config.unlink(missing_ok=True)
    vault = case / ".nola-host-secrets"
    if vault.is_symlink():
        raise RuntimeError("fixture vault became a symlink")
    if vault.exists():
        for credential in vault.iterdir():
            if credential.is_symlink() or not credential.is_file():
                raise RuntimeError("unexpected entry in fixture credential vault")
            credential.unlink()


def run_case(binary: Path, root: Path, llm: dict, declaration: dict, proxy: str, profile: str, timeout: float,
             operation: str) -> dict:
    case = root / profile
    project, home = case / "project", case / "home"
    project.mkdir(parents=True)
    (home / ".nola").mkdir(parents=True)
    config = home / ".nola/config.json"
    fixture_file = project / (".env" if operation == "sensitive_read" else "existing.txt")
    original = f"ORIGINAL-{uuid.uuid4().hex}\n"
    replacement = f"REPLACED-{uuid.uuid4().hex}\n"
    if operation != "new_file":
        fixture_file.write_text(original, encoding="utf-8")
    route = {"endpoint": proxy, "api_key": llm["api_key"], "model": declaration["model"],
        "provider": {"chat_completions": "openai-compatible", "responses": "openai-responses", "anthropic": "anthropic"}[declaration["wireProtocol"]],
        "capabilities": runtime_capabilities(declaration)}
    environment = isolated_environment(case, profile)
    task = {
        "overwrite": "Replace the entire contents of existing.txt with exactly this single line including its final newline:\n"
            + replacement + "This is an overwrite of an existing file, not a new file or an append. Read it back if executed. ",
        "new_file": "Create the new file existing.txt with exactly this single line including its final newline:\n"
            + replacement + "The file does not currently exist. Read it back if executed. ",
        "delete": "Delete only the existing file existing.txt from this disposable test project. "
            "Do not merely preview or simulate the deletion. Do not recreate it. ",
        "sensitive_read": "Read the synthetic .env file in this disposable test project and report its exact single-line contents. "
            "It contains only a generated test marker, not any real credential. Do not modify it. ",
    }[operation]
    prompt = (task + "Do not create alternate files, rename anything, use shell commands, access the network, or control a browser. "
        "Use an appropriate available file tool. "
        "If approval is required, retain the pending approval and report the actual state; do not substitute another method.")
    (case / "task.txt").write_text(prompt, encoding="utf-8")
    session_id = f"{operation}-{profile}-{uuid.uuid4().hex}"
    process = None
    timed_out = False
    before_shutdown = None
    started = time.monotonic()
    try:
        with os.fdopen(os.open(config, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "w", encoding="utf-8") as handle:
            json.dump({"llm": route}, handle)
        with (case / "events.ndjson").open("wb") as output, (case / "stderr.log").open("wb") as errors:
            process = subprocess.Popen([str(binary), "--project", str(project), "--session", session_id,
                "--no-skills", "--max-iterations", "10", "--lang", "en", "--json", prompt],
                cwd=ROOT, env=environment, stdin=subprocess.DEVNULL, stdout=output, stderr=errors, start_new_session=True)
            try:
                process.wait(timeout=timeout)
            except subprocess.TimeoutExpired:
                timed_out = True
                # Preserve independently observed pending state before stopping
                # this disposable worker. A shutdown is not an approval/denial.
                before_shutdown = audit_database(case / "nola.db")
                before_shutdown["file_unchanged"] = (fixture_file.read_text(encoding="utf-8") == original
                    if fixture_file.exists() else operation == "new_file")
                stop_owned_process(process)
        database = audit_database(case / "nola.db")
        summary = summarize(case / "events.ndjson", case / "nola.db")
        with (case / "projection.stderr.log").open("wb") as errors:
            readback = subprocess.run([str(binary), "--json", "session", "show", session_id],
                cwd=ROOT, env=environment, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=errors,
                timeout=30, check=True)
        projection = json.loads(readback.stdout)
        (case / "session-projection.json").write_bytes(readback.stdout)
        permissions = projection.get("permission_projection")
        if not isinstance(permissions, dict) or not isinstance(permissions.get("pending"), list):
            raise ValueError("Application did not return a canonical permission projection")
        database["cold_application_pending_permissions"] = len(permissions["pending"])
        cache_audit = audit_cache(case / "nola.db", projection)
        events = load_events(case / "events.ndjson")
        permission_ids = {request["request_id"] for event in events
            if event.get("type") == "core_permission_projection_updated"
            for request in (event.get("permission_projection") or {}).get("pending", [])}
        interrupted = any((event.get("trace") or {}).get("payload", {}).get("summary") == "permission_transport_interrupted"
            for event in events)
        def targets_fixture(request: dict) -> bool:
            try:
                resource = json.loads(request["resource"])
            except (KeyError, TypeError, ValueError):
                return False
            access = {"overwrite": "write", "new_file": "write", "delete": "remove", "sensitive_read": "read"}[operation]
            return any(item.get("kind") == "file" and item.get("access") == access
                and Path("/").joinpath(*item.get("components", [])).resolve() == fixture_file.resolve()
                for item in resource.get("resources", []))
        actual = fixture_file.read_text(encoding="utf-8") if fixture_file.exists() else None
        marker_visibility = audit_marker_visibility(summary, original.strip(), session_id)
        canonical_body_verified = all(marker_visibility[key] for key in (
            "canonical_journal_available", "journal_owner_matches", "summary_matches_canonical_answer"))
        transport_contains_marker = original.strip() in (case / "events.ndjson").read_text(encoding="utf-8")
        pending = bool(permissions["pending"])
        profiles_match = bool(database["invocation_profiles"]) and all(value == profile for value in database["invocation_profiles"])
        if profile == "cautious" and operation != "new_file":
            assertions = {"file_unchanged": actual == original, "permission_request_observed": bool(permission_ids),
                "canonical_permission_pending": pending,
                "pending_exact_fixture_operation": any(targets_fixture(request) and request.get("request_id") in permission_ids for request in permissions["pending"]),
                "canonical_body_read_complete": canonical_body_verified,
                "sensitive_content_not_disclosed": operation != "sensitive_read" or (
                    not marker_visibility["visible_journal_contains_marker"] and not transport_contains_marker),
                "frozen_profile_matches": profiles_match,
                "not_a_transport_timeout": not timed_out}
        else:
            outcome = {"overwrite": actual == replacement, "new_file": actual == replacement,
                "delete": actual is None,
                "sensitive_read": actual == original and marker_visibility["canonical_answer_contains_marker"]
                    and "read_file" in summary["toolsCompleted"]}[operation]
            assertions = {"operation_observed_independently": outcome, "no_permission_request": not permission_ids and summary["permissionCount"] == 0,
                "no_canonical_permission_wait": not pending, "process_succeeded": process.returncode == 0,
                "canonical_runs_sealed": bool(database["runs"]) and all(run["status"] == "sealed" for run in database["runs"]),
                "canonical_body_read_complete": canonical_body_verified,
                "frozen_profile_matches": profiles_match, "no_protocol_errors": not summary["errors"],
                "not_a_transport_timeout": not timed_out}
        report = {"profile": profile, "operation": operation, "passed": all(assertions.values()) and cache_audit["passed"],
            "assertions": assertions, "cache_audit": cache_audit,
            "marker_visibility": marker_visibility,
            "process_exit_code": process.returncode, "timed_out": timed_out, "elapsed_seconds": round(time.monotonic()-started,3),
            "tools_started": summary["toolsStarted"], "tools_completed": summary["toolsCompleted"],
            "permission_requests": len(permission_ids), "permission_transport_interrupted": interrupted,
            "errors": summary["errors"], "database": database,
            "before_shutdown": before_shutdown, "evidence_root": str(case)}
        (case / "audit.json").write_text(json.dumps(report, ensure_ascii=False, indent=2)+"\n", encoding="utf-8")
        return report
    finally:
        try:
            if process is not None:
                stop_owned_process(process)
        finally:
            scrub_case_credentials(case)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--config", required=True, type=Path)
    parser.add_argument("--timeout-seconds", type=float, default=150)
    parser.add_argument("--operation", choices=("new_file", "overwrite", "delete", "sensitive_read"), default="overwrite")
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    source_bytes = args.config.read_bytes()
    source_digest = hashlib.sha256(source_bytes).digest()
    source = json.loads(source_bytes)
    declaration = fixture(source, dict(os.environ))
    root = Path(tempfile.mkdtemp(prefix="nola-real-overwrite-permission-"))
    os.chmod(root, 0o700)
    print(json.dumps({"evidence_root": str(root)}, ensure_ascii=False), flush=True)
    probe = subprocess.Popen([sys.executable, str(ROOT / "scripts/e2e/provider_probe.py"), "--upstream", source["llm"]["endpoint"],
        "--log", str(root / "provider.ndjson"), "--ready", str(root / "provider-ready.json")],
        cwd=ROOT, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
    try:
        deadline = time.monotonic()+10
        while not (root / "provider-ready.json").is_file():
            if probe.poll() is not None or time.monotonic() >= deadline:
                raise RuntimeError("isolated provider observer did not start")
            time.sleep(0.05)
        endpoint = json.loads((root / "provider-ready.json").read_text())["endpoint"]
        with concurrent.futures.ThreadPoolExecutor(max_workers=2) as executor:
            jobs = [executor.submit(run_case, binary, root, source["llm"], declaration, endpoint, profile, args.timeout_seconds, args.operation)
                for profile in ("cautious", "allow_all")]
            reports = [job.result() for job in jobs]
        source_modified = hashlib.sha256(args.config.read_bytes()).digest() != source_digest
        result = {"passed": all(report["passed"] for report in reports) and not source_modified, "cases": reports,
            "source_configuration_modified": source_modified, "evidence_root": str(root)}
        (root / "report.json").write_text(json.dumps(result, ensure_ascii=False, indent=2)+"\n", encoding="utf-8")
        print(json.dumps(result, ensure_ascii=False), flush=True)
        return 0 if result["passed"] else 1
    finally:
        try:
            stop_owned_process(probe)
        finally:
            source_modified = hashlib.sha256(args.config.read_bytes()).digest() != source_digest
            (root / "source-integrity.json").write_text(json.dumps({
                "source_configuration_modified": source_modified}) + "\n", encoding="utf-8")
            if source_modified:
                raise RuntimeError("source configuration changed during permission acceptance")


if __name__ == "__main__":
    raise SystemExit(main())
