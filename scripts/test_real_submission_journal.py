#!/usr/bin/env python3
"""Real-provider Host acceptance through source receipts and public journal APIs.

Uses an isolated NOLA_HOME/database, never rewrites HOME or the user's settings.
This tests the product HTTP path, not a direct provider inference shortcut.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import socket
import sqlite3
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
import uuid
sys.path.insert(0, str(Path(__file__).resolve().parent / "e2e"))
from e2e.real_overwrite_permission import scrub_case_credentials


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def count(value):
    require(isinstance(value, str) and value.isascii() and value.isdecimal()
            and str(int(value)) == value and int(value) <= 2**64 - 1, "Invalid decimal u64")
    return int(value)


def digest(value):
    require(isinstance(value, str) and len(value) == 64
            and all(c in "0123456789abcdef" for c in value), "Invalid digest")
    return value


def typed(value, keys):
    require(isinstance(value, dict) and set(value) == set(keys), "Unexpected typed fields")
    return {key: value[key] for key in keys}


def journal_identity(value):
    result = typed(value, ("source_store_epoch", "conversation_id", "run_id", "assistant_message_id"))
    require(all(isinstance(v, str) and v.strip() for v in result.values()), "Empty journal identity")
    return result


def cursor(value):
    result = typed(value, ("identity", "sequence", "record_digest"))
    result["identity"] = journal_identity(result["identity"])
    count(result["sequence"])
    digest(result["record_digest"])
    return result


def domain_digest(tag, value):
    # Match serde_json typed serialization, not the HTTP object's key order.
    raw = json.dumps(value, ensure_ascii=False, separators=(",", ":"), allow_nan=False).encode("utf-8")
    return hashlib.sha256(tag + len(raw).to_bytes(8, "big") + raw).hexdigest()


def manifest_entry(value):
    if value.get("kind") == "assistant":
        entry = typed(value, ("kind", "through", "ordinal", "created_at_ms", "state", "completed_at_ms"))
        entry["through"] = cursor(entry["through"])
        for field in ("ordinal", "created_at_ms", "completed_at_ms"):
            require(type(entry[field]) is int and 0 <= entry[field] <= 2**63 - 1, "Invalid assistant timestamp/ordinal")
        require(entry["state"] in ("completed", "failed", "cancelled"), "Unsealed assistant")
        return entry
    entry = typed(value, ("kind", "task_id", "receipt_id", "parent_turn_id", "child_session_id", "child_run_id",
                          "input_watermark", "completion_watermark", "status", "receipt_digest", "result_digest"))
    require(entry["kind"] == "child_receipt" and entry["status"] in ("completed", "failed", "cancelled"), "Invalid child receipt")
    require(all(isinstance(entry[k], str) and entry[k] for k in ("task_id", "receipt_id", "parent_turn_id")), "Missing child owner")
    for field in ("child_session_id", "child_run_id"):
        require(entry[field] is None or isinstance(entry[field], str), "Invalid child identity")
    for field in ("input_watermark", "completion_watermark"):
        require(entry[field] is None or type(entry[field]) is int and -2**63 <= entry[field] < 2**63, "Invalid child watermark")
    digest(entry["receipt_digest"])
    digest(entry["result_digest"])
    return entry


class Host:
    def __init__(self, binary: Path, config: Path):
        self.directory = Path(tempfile.mkdtemp(prefix="nola-real-submission-"))
        config_dir = self.directory / ".nola"
        config_dir.mkdir(mode=0o700)
        # Private fixture credential copy is removed on exit; evidence stays.
        self.config = config_dir / "config.json"
        with self.config.open("x", encoding="utf-8") as handle:
            os.chmod(self.config, 0o600)
            handle.write(config.read_text(encoding="utf-8"))
        self.binary = binary
        self.http = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        self.process: subprocess.Popen | None = None
        self.log = (self.directory / "host.log").open("ab")
        try:
            self.start()
        except BaseException:
            self.close()
            raise

    def start(self):
        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1", 0))
            port = reservation.getsockname()[1]
        self.base = f"http://127.0.0.1:{port}"
        environment = os.environ.copy()
        environment.pop("NOLA_CLI_PROJECT_ROOT", None)
        environment.update(NOLA_HOME=str(self.directory), NOLA_DB_PATH=str(self.directory / "acceptance.sqlite"),
                           NOLA_CLI_INTERACTION_MODE="never", NO_PROXY="127.0.0.1,localhost", no_proxy="127.0.0.1,localhost")
        self.process = subprocess.Popen([str(self.binary), "web", "--project", str(self.directory), "--no-skills", "--host", "127.0.0.1",
                                         "--port", str(port), "--web-root", str(self.directory)],
                                        cwd=self.directory, env=environment, stdout=self.log, stderr=self.log)
        deadline = time.monotonic() + 45
        while time.monotonic() < deadline:
            if self.process.poll() is not None:
                raise RuntimeError("Host exited before readiness; inspect private evidence directory")
            try:
                self.host_id = self.json("/api/health")["host"]["id"]
                return
            except (OSError, RuntimeError):
                time.sleep(0.1)
        raise RuntimeError("Host readiness timed out")

    def stop(self):
        if self.process and self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=10)

    def close(self):
        self.stop()
        self.log.close()
        self.config.unlink(missing_ok=True)
        scrub_case_credentials(self.directory)

    def request(self, path, body=None, timeout=30):
        data = None if body is None else json.dumps(body, ensure_ascii=False).encode()
        request = urllib.request.Request(self.base + path, data=data,
                                         headers={"Content-Type": "application/json"} if data else {})
        try:
            return self.http.open(request, timeout=timeout)
        except urllib.error.HTTPError as error:
            # Never print provider payloads/credentials from a failure body.
            error.close()
            raise RuntimeError(f"HTTP {error.code}: {path}") from None

    def json(self, path, body=None):
        with self.request(path, body) as response:
            return json.load(response)

    def operation(self, path, body):
        envelope = self.json(path, body)
        assert envelope["source_host_id"] == self.host_id
        return envelope["result"]

    def prepare(self, conversation, prompt):
        submission = str(uuid.uuid4())
        identity = dict(source_host_id=self.host_id, client_id="real-submission-acceptance",
                        conversation_id=conversation, submission_id=submission,
                        user_message_id=submission + ":user", assistant_message_id=submission + ":assistant")
        plan = self.operation("/api/agent/tool-plan/prepare", dict(expected_host_id=self.host_id,
            client_id=identity["client_id"], session_id=conversation))
        body = dict(conversationId=conversation, expected_host_id=self.host_id, client_id=identity["client_id"],
                    submission_id=submission, user_message_id=identity["user_message_id"],
                    assistant_message_id=identity["assistant_message_id"], execution_backend="nola", tool_plan_ref=plan["reference"],
                    messages=[dict(role="user", content=prompt)], max_iterations=4)
        prepared = self.operation("/api/chat/agent/v2/submission", dict(operation="prepare", identity=identity, request=body))
        assert prepared["identity"] == identity
        body["request_digest"] = prepared["request_digest"]
        return identity, body

    def stream(self, body):
        events = []
        deadline = time.monotonic() + 240
        with self.request("/api/chat/agent/v2/stream", body, timeout=240) as response:
            for line in response:
                if time.monotonic() >= deadline:
                    raise RuntimeError("Host stream exceeded total acceptance deadline")
                if line.startswith(b"data:"):
                    payload = line[5:].strip()
                    if payload != b"[DONE]":
                        events.append(json.loads(payload))
        return events

    def observe(self, identity):
        return self.operation("/api/chat/agent/v2/observe", dict(operation="observe",
            conversation_id=identity["conversation_id"], expected_host_id=self.host_id,
            client_id=identity["client_id"], expected_run_id=identity["submission_id"],
            assistant_message_id=identity["assistant_message_id"]))

    def completed(self, identity, request_digest):
        observed = self.observe(identity)
        require(observed["client_id"] == identity["client_id"] and observed["conversation_id"] == identity["conversation_id"]
                and observed["run_id"] == identity["submission_id"]
                and observed["assistant_message_id"] == identity["assistant_message_id"], "Observed owner differs")
        receipt = observed["submission_receipt"]
        require(receipt["protocol_version"] == 1 and receipt["identity"] == identity
                and receipt["request_digest"] == request_digest and receipt["phase"] == "admitted", "Receipt differs")
        seal = observed["terminal_seal"]
        require(isinstance(seal, dict) and observed["status"] == "sealed", "Run did not durably seal")
        typed(seal, ("protocol_version", "identity", "core_session_id", "transition_id", "checkpoint_revision", "core_revision", "event_cursor",
                     "result", "result_digest", "completed_at_ms", "admission", "entry_count", "heads_count", "child_receipt_count", "manifest_digest", "seal_digest"))
        result = typed(seal["result"], ("result_id", "run_id", "conversation_id", "originating_submission_id", "input",
            "decided_at_revision", "decision_event_cursor", "requirement_revision", "evidence_frontier", "value"))
        require(result["run_id"] == identity["submission_id"] and result["conversation_id"] == identity["conversation_id"]
            and result["originating_submission_id"] == identity["submission_id"], "Result ownership differs")
        require(isinstance(result["result_id"], str) and result["result_id"].strip(), "Empty result identity")
        result["input"] = typed(result["input"], ("watermark", "applications_digest"))
        count(result["input"]["watermark"])
        digest(result["input"]["applications_digest"])
        for field in ("decided_at_revision", "decision_event_cursor", "requirement_revision", "evidence_frontier"):
            count(result[field])
        require(count(result["decided_at_revision"]) <= count(seal["core_revision"])
            and count(result["decision_event_cursor"]) <= count(seal["event_cursor"]), "Result exceeds terminal frontier")
        kind = result["value"]["kind"]
        require(kind in ("answered", "action_completed"), "Run did not durably complete successfully: " + kind)
        refs = "answer_part_refs" if kind == "answered" else "requirement_acceptance_refs"
        result["value"] = typed(result["value"], ("kind", refs))
        require(isinstance(result["value"][refs], list) and result["value"][refs]
            and all(isinstance(item, str) and item for item in result["value"][refs]), "Empty result evidence")
        require(len(set(result["value"][refs])) == len(result["value"][refs]), "Duplicate result evidence")
        if kind == "action_completed":
            result["value"][refs] = sorted(result["value"][refs])
        require(domain_digest(b"NOLA-RUN-RESULT-V2", result) == digest(seal["result_digest"]), "Result digest differs")
        root = journal_identity(seal["identity"])
        require(root["conversation_id"] == identity["conversation_id"] and root["run_id"] == identity["submission_id"]
                and root["assistant_message_id"] == identity["assistant_message_id"], "Seal root differs")
        admission = cursor(seal["admission"])
        admitted = cursor(receipt["admission"])
        require(cursor(observed["admission"]) == admitted, "Observed admission and receipt differ")
        require(admission["identity"] == root == admitted["identity"] and count(admission["sequence"]) > 0
                and count(admitted["sequence"]) >= count(admission["sequence"]), "Seal/admission epoch or frontier differs")
        if admitted["sequence"] == admission["sequence"]:
            require(admitted == admission, "Admission digest differs")
        require(seal["protocol_version"] == 1 and isinstance(seal["core_session_id"], str) and seal["core_session_id"].strip()
                and isinstance(seal["transition_id"], str) and seal["transition_id"].strip()
                and type(seal["completed_at_ms"]) is int and 0 <= seal["completed_at_ms"] <= 2**63 - 1, "Invalid seal")
        require(count(seal["checkpoint_revision"]) > 0 and count(seal["core_revision"]) > 0 and count(seal["heads_count"]) > 0, "Empty terminal frontier")
        for field in ("event_cursor", "entry_count", "child_receipt_count"):
            count(seal[field])
        seal_tuple = [seal["protocol_version"], root, seal["core_session_id"], seal["transition_id"],
                      seal["checkpoint_revision"], seal["core_revision"], seal["event_cursor"], seal["result_digest"], seal["completed_at_ms"], admission,
                      seal["entry_count"], seal["heads_count"], seal["child_receipt_count"], digest(seal["manifest_digest"])]
        require(domain_digest(b"NOLA-RUN-TERMINAL-SEAL-V2", seal_tuple) == digest(seal["seal_digest"]), "Seal digest differs")
        entries, after = [], 0
        while True:
            page = self.operation("/api/chat/agent/v2/journal", dict(operation="terminal_manifest", seal=seal,
                after_entry=str(after), max_records=64, max_encoded_bytes=131072))
            require(page["seal"] == seal and count(page["after_entry"]) == after, "Manifest page changed frontier")
            batch = [manifest_entry(value) for value in page["entries"]]
            require(len(batch) <= 64 and count(page["next_entry"]) == after + len(batch), "Invalid manifest page bounds")
            entries.extend(batch)
            after += len(batch)
            require(after <= count(seal["entry_count"]) and type(page["complete"]) is bool
                    and page["complete"] == (after == count(seal["entry_count"])), "Incomplete manifest")
            if page["complete"]:
                break
            require(bool(batch), "Manifest made no progress")
        require(domain_digest(b"NOLA-RUN-TERMINAL-MANIFEST-V2", entries) == seal["manifest_digest"], "Manifest digest differs")
        owners, tasks, ordinal = set(), set(), -1
        for entry in entries:
            if entry["kind"] == "assistant":
                through = entry["through"]
                owner = through["identity"]
                require(not tasks and all(owner[k] == root[k] for k in ("source_store_epoch", "conversation_id", "run_id"))
                        and count(through["sequence"]) > 0 and entry["ordinal"] > ordinal
                        and owner["assistant_message_id"] not in owners, "Manifest owner/order differs")
                ordinal = entry["ordinal"]
                owners.add(owner["assistant_message_id"])
            else:
                require(entry["task_id"] not in tasks, "Duplicate child receipt")
                tasks.add(entry["task_id"])
        require(len(owners) == count(seal["heads_count"]) and len(tasks) == count(seal["child_receipt_count"])
                and len(entries) == len(owners) + len(tasks) and root["assistant_message_id"] in owners, "Manifest counts/root differ")
        return seal, entries

    def attempt_frontier(self, identity):
        # Read only this isolated fixture database. This is durable attempt
        # evidence, not a claim to observe every physical provider HTTP call.
        with sqlite3.connect((self.directory / "acceptance.sqlite").as_uri() + "?mode=ro", uri=True) as database:
            rows = database.execute("SELECT id, record_json FROM run_attempts WHERE run_id=? ORDER BY id",
                                    (identity["submission_id"],)).fetchall()
        require(bool(rows), "No durable source model attempt evidence")
        return [(key, hashlib.sha256(record.encode()).hexdigest()) for key, record in rows]

    def text(self, entries):
        def journal(**body):
            return self.operation("/api/chat/agent/v2/journal", body)
        output = []
        sections = ["transcript", "tasks", "skills", "model_attempts"]
        for entry in entries:
            if entry["kind"] != "assistant":
                continue
            head = entry["through"]
            snapshot = journal(operation="open_snapshot", at=head)
            try:
                require(snapshot["at"] == head, "Snapshot is not the sealed watermark")
                parts, after = [], None
                while True:
                    page = journal(operation="parts", snapshot=snapshot, after_part_id=after,
                                   max_records=32, max_encoded_bytes=65536)
                    # AssistantPartPage has no echoed `at`; the authenticated
                    # query is pinned by the exact snapshot handle above.
                    ids = [part["part_id"] for part in page["parts"]]
                    require(len(ids) <= 32 and ids == sorted(set(ids))
                            and (after is None or all(part_id > after for part_id in ids)), "Part page order differs")
                    parts.extend(page["parts"])
                    next_part = page["next_part_id"]
                    if next_part is None:
                        break
                    # Directory progress may cover only retired Parts and
                    # legitimately return an empty page at this fixed snapshot.
                    require(isinstance(next_part, str) and next_part and (after is None or next_part > after)
                            and all(part_id <= next_part for part_id in ids), "Part page made no progress")
                    after = next_part
                require(len({part["part_id"] for part in parts}) == len(parts), "Duplicate Part")
                for part in parts:
                    require(part["identity"] == head["identity"] and part["order_key"]["section"] in sections, "Part owner/section differs")
                    count(part["order_key"]["position"])
                for part in sorted(parts, key=lambda part: (sections.index(part["order_key"]["section"]), count(part["order_key"]["position"]), part["part_id"])):
                    if part["metadata"]["kind"] != "text" or part["metadata"]["data"]["visibility"] != "public":
                        continue
                    require(part["metadata"]["data"].get("review") in (None, "approved"), "Undisclosed review body")
                    markdown = [body for body in part["bodies"] if body["field"] == "markdown"]
                    require(len(markdown) == 1, "Text lacks its unique markdown body")
                    content = markdown[0]
                    offset, total = 0, count(content["utf8_bytes"])
                    while offset < total:
                        chunk = journal(operation="range", snapshot=snapshot, part_id=part["part_id"],
                                        content=content, offset_bytes=str(offset), limit_bytes=32768)
                        require(count(chunk["offset_bytes"]) == offset and count(chunk["total_utf8_bytes"]) == total, "Range watermark differs")
                        next_offset = count(chunk["next_offset_bytes"])
                        require(offset < next_offset <= total and next_offset - offset == len(chunk["text"].encode("utf-8"))
                                and next_offset - offset <= 32768 and type(chunk["end_of_content"]) is bool
                                and chunk["end_of_content"] == (next_offset == total), "Invalid UTF-8 range")
                        output.append(chunk["text"])
                        offset = next_offset
                    assert offset == total
            finally:
                journal(operation="release_snapshot", snapshot=snapshot)
        return "".join(output)


def main():
    require(not sys.flags.optimize, "This acceptance script must not run with Python -O/PYTHONOPTIMIZE")
    parser = argparse.ArgumentParser()
    parser.add_argument("binary", type=Path)
    parser.add_argument("--config", type=Path, required=True)
    args = parser.parse_args()
    host = Host(args.binary.resolve(strict=True), args.config.resolve(strict=True))
    print(f"Isolated evidence: {host.directory}", flush=True)
    started = time.monotonic()
    try:
        conversation = "real-submission-" + str(uuid.uuid4())
        marker = "NOLA_" + uuid.uuid4().hex[:12]
        identity, body = host.prepare(conversation, f"Reply with exactly this token: {marker}")
        events = host.stream(body)
        admitted = next(index for index, event in enumerate(events)
                        if event.get("type") == "submission_receipt" and event["receipt"]["phase"] == "admitted")
        first_journal = next(index for index, event in enumerate(events) if event.get("type") == "assistant_journal_committed_v1")
        assert admitted < first_journal
        receipt = events[admitted]["receipt"]
        require(receipt["identity"] == identity and receipt["request_digest"] == body["request_digest"], "Stream receipt differs")
        seal, manifest = host.completed(identity, body["request_digest"])
        first = events[first_journal]
        first_cursor = cursor(first["through"])
        require(first["source_host_id"] == host.host_id and first["session_id"] == conversation
                and first_cursor["identity"] == seal["identity"], "First journal does not belong to the admitted root")
        require(host.text(manifest).strip() == marker, "Sealed answer must equal the exact token")
        attempt_frontier = host.attempt_frontier(identity)
        duplicate = host.stream(body)
        assert duplicate and all(event.get("type") == "submission_receipt" for event in duplicate)
        for event in duplicate:
            require(event["receipt"]["identity"] == identity and event["receipt"]["request_digest"] == body["request_digest"]
                    and event["receipt"]["phase"] == "admitted", "Duplicate receipt differs")
        duplicate_seal, duplicate_manifest = host.completed(identity, body["request_digest"])
        require((duplicate_seal, duplicate_manifest) == (seal, manifest), "Duplicate changed terminal frontier")
        require(host.attempt_frontier(identity) == attempt_frontier, "Duplicate changed durable model attempts")
        # Reboot the real Host: no in-process presenter or cached checkpoint can
        # supply the receipt, terminal seal, or displayed answer after this.
        source = host.host_id
        host.stop()
        host.start()
        assert host.host_id == source
        cold_seal, cold_manifest = host.completed(identity, body["request_digest"])
        require((cold_seal, cold_manifest) == (seal, manifest), "Cold terminal evidence changed")
        require(host.text(cold_manifest).strip() == marker, "Cold sealed answer differs")
        require(host.attempt_frontier(identity) == attempt_frontier, "Restart changed durable model attempts")
        second, second_body = host.prepare(conversation, "请仅重复上一轮的验证 token，不要添加解释。")
        host.stream(second_body)
        _, second_manifest = host.completed(second, second_body["request_digest"])
        require(host.text(second_manifest).strip() == marker, "Natural follow-up must retain exact canonical context")
        pending, late_body = host.prepare("cancel-" + conversation, "This cancelled submission must never execute.")
        cancelled = host.operation("/api/chat/agent/v2/submission", dict(operation="cancel", identity=pending,
                                    request_digest=late_body["request_digest"]))
        require(cancelled["phase"] == "cancelled" and cancelled["identity"] == pending
                and cancelled["request_digest"] == late_body["request_digest"], "Cancellation receipt differs")
        late = host.stream(late_body)
        assert len(late) == 1 and late[0]["receipt"] == cancelled
        evidence = dict(passed=True, elapsed_seconds=round(time.monotonic() - started, 3),
                        checks=["admission-before-journal", "typed-seal-and-paged-manifest-digests", "exact-sealed-utf8-answer",
                                "duplicate-unchanged-durable-model-attempt-frontier", "host-restart-recovery",
                                "multilingual-follow-up", "cancel-before-late-post"],
                        duplicate_evidence={"durable_model_attempt_frontier": attempt_frontier,
                            "physical_provider_http_calls_observed": False},
                        terminal_seal=seal)
        (host.directory / "summary.json").write_text(json.dumps(evidence, indent=2) + "\n")
        print(json.dumps(evidence), flush=True)
    finally:
        host.close()


if __name__ == "__main__":
    main()
