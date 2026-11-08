#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Bounded 192.* model recovery after one disclosed tool-name corruption."""
import argparse
import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import secrets
import tempfile
from urllib.parse import urlparse

from run_tool_reference_compaction_e2e import canonical_results, run_prompt
from test_real_cli_commit_followup import assert_completed_turn

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("web_unknown_fault", ROOT / "web/tests/test_real_llm_web_agent_e2e.py")
web_fault = importlib.util.module_from_spec(spec)
spec.loader.exec_module(web_fault)


class AuditedFaultProxy(web_fault.UnknownToolFaultProxy):
    def __init__(self, *args):
        super().__init__(*args)
        self.injections = []
        self._fault_response = False
        self._fault_index = None

    def _rewrite(self, body, content_type):
        self._fault_response = self.original_tool_name is None
        try:
            return super()._rewrite(body, content_type)
        finally:
            self._fault_response = False

    def _rewrite_event(self, event):
        before = copy.deepcopy(event)
        super()._rewrite_event(event)
        if before == event:
            if self._fault_response and self.injections:
                for choice in event.get("choices", []):
                    for call in choice.get("delta", {}).get("tool_calls", []):
                        if call.get("index") == self._fault_index and call.get("function", {}).get("name"):
                            self.injections[0]["original_name"] += call["function"].pop("name")
            return
        changes = []
        restored = copy.deepcopy(event)
        for old_choice, new_choice, restored_choice in zip(before.get("choices", []), event.get("choices", []), restored.get("choices", [])):
            key = "delta" if old_choice.get("delta") else "message"
            for old, new, reset in zip(old_choice.get(key, {}).get("tool_calls", []),
                                       new_choice.get(key, {}).get("tool_calls", []),
                                       restored_choice.get(key, {}).get("tool_calls", [])):
                if old != new:
                    changes.append({"original_name": old["function"]["name"],
                                    "replacement_name": new["function"]["name"],
                                    "call_id": old.get("id"),
                                    "arguments_sha256": hashlib.sha256(old["function"].get("arguments", "").encode()).hexdigest()})
                    self._fault_index = old.get("index")
                    reset["function"]["name"] = old["function"]["name"]
        assert len(changes) == 1 and restored == before, "Fault changed more than a single tool name"
        self.injections.extend(changes)


def verify_recovery(events, results, marker, injections, answer=""):
    assert len(injections) == 1, "Exactly one real tool-name injection is required"
    assert_completed_turn(events)
    calls = [event["trace"]["payload"].get("tool_name") for event in events
             if event.get("trace", {}).get("kind") in ("tool.started", "tool.completed")]
    allowed = {"tool_search", "read_file", "nola_unknown_tool_probe"}
    assert set(calls) <= allowed, f"Unexpected tool bypass: {set(calls) - allowed}"
    completed = [event["trace"] for event in events if event.get("trace", {}).get("kind") == "tool.completed"]
    failed = [trace for trace in completed if trace["status"] == "failed"
              and trace["payload"].get("tool_name") == "nola_unknown_tool_probe"]
    assert failed, "Core did not reject the injected tool call"
    rejection = failed[0]
    call_id = rejection["payload"]["call_id"]
    assert call_id.endswith("::" + injections[0]["call_id"])
    rejected = [result for result in results if result.get("call_id") == call_id]
    assert len(rejected) == 1 and rejected[0]["is_error"] is True
    assert rejected[0]["output"]["error"] == "tool_not_exposed_in_model_step"
    assert rejected[0]["output"]["tool_execution_skipped"] is True
    successful = {trace["payload"]["call_id"] for trace in completed
                  if trace["payload"].get("tool_name") == "read_file" and trace["status"] == "succeeded"
                  and trace["run_id"] == rejection["run_id"] and trace["sequence"] > rejection["sequence"]}
    reads = [result for result in results if result.get("name") == "read_file"
             and result.get("is_error") is False and result.get("call_id") in successful]
    assert any(marker in json.dumps(result["output"]) for result in reads), "No actual successful marker read"
    assert marker in answer, "Final assistant answer omitted the exact verification code"
    assert not any(result.get("name") not in allowed for result in results), "Unexpected canonical tool"
    return {"read_result_ids": [result["id"] for result in reads], "calls": calls}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, default=ROOT / "target/debug/nola")
    parser.add_argument("--config", type=Path, default=Path.home() / ".nola/config.json")
    args = parser.parse_args()
    source = json.loads(args.config.read_text())
    llm = source["llm"]
    host = urlparse(llm["endpoint"]).hostname or ""
    assert host.startswith("192."), "A real 192.* model endpoint is required"
    root = Path(tempfile.mkdtemp(prefix="nola-unknown-tool-recovery-"))
    os.chmod(root, 0o700)
    project = root / "project"
    project.mkdir()
    (project / ".git").mkdir()
    marker = "RECOVERY-" + secrets.token_hex(16)
    content = "Isolated read-only verification record: " + marker + "\n"
    (project / "record.txt").write_text(content)
    home = root / "home"
    (home / ".nola").mkdir(parents=True)
    config = home / ".nola/config.json"
    database = root / "state.db"
    session = "unknown-tool-" + secrets.token_hex(8)
    proxy = AuditedFaultProxy(llm["endpoint"], root / "provider.ndjson")
    settings = {"llm": {**llm, "endpoint": proxy.endpoint}, "allowed_dirs": [str(project)],
                "network_policy": "deny", "max_iterations": 10}
    if "model_capabilities" in source:
        settings["model_capabilities"] = source["model_capabilities"]
    with open(config, "x", opener=lambda path, flags: os.open(path, flags, 0o600)) as handle:
        json.dump(settings, handle)
    report = {"passed": False, "session": session, "endpoint_host": host,
              "model": llm["model"], "fault": "single real-model tool name corruption; not a naturally occurring model error"}
    print(f"Evidence: {root}", flush=True)
    proxy.start()
    try:
        env = {**os.environ, "NOLA_HOME": str(home), "NOLA_DB_PATH": str(database), "NOLA_SKIP_ONBOARDING": "1"}
        command = [str(args.binary.resolve()), "--project", str(project), "--session", session,
                   "--yes", "--json", "--max-iterations", "10"]
        events = run_prompt(command, "Read record.txt with read_file and report its exact verification code. "
                            "Use tool_search if you need to discover an available tool. Do not change files, "
                            "execute shell commands, use a browser, or access the network.",
                            project, env, root / "events.ndjson", 210)
        results = canonical_results(database, session)
        import sqlite3
        run_id = next(event["trace"]["run_id"] for event in events if event.get("trace", {}).get("kind") == "run.started")
        with sqlite3.connect(database.as_uri() + "?mode=ro", uri=True) as db:
            parts = db.execute("SELECT content_json FROM message_parts WHERE message_id=? AND kind='text' AND state='completed'", (run_id + ":assistant",)).fetchall()
        answer = "\n".join(json.loads(raw)["text"] for raw, in parts if raw)
        report.update(verify_recovery(events, results, marker, proxy.injections, answer))
        assert (project / "record.txt").read_text() == content
        assert {p.name for p in project.iterdir()} == {".git", "record.txt"}
        assert proxy.request_count >= 2, "Recovery requires another real model request"
        report["passed"] = True
    except Exception as error:
        report["error"] = f"{type(error).__name__}: {error}"
    finally:
        proxy.close()
        config.unlink(missing_ok=True)
        report["injections"] = proxy.injections
        report["provider_requests"] = proxy.request_count
        (root / "summary.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report))
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
