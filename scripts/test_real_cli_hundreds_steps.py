#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Hundreds of real, causally dependent CLI model/tool rounds in one Run.

The selected provider is forwarded transparently by the existing acceptance
relay. Fixtures, credentials, canonical outcomes and independent file oracles
remain separate. A short smoke is explicitly not hundreds-step acceptance.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import secrets
import signal
import sqlite3
import subprocess
import tempfile
import time

from test_real_cli_long_tasks import (
    apply_config, command, fixture, observed_provider, require_completed, selected_route,
)
from canonical_run_audit import audit_runs


def write_json(path, value):
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps(value, ensure_ascii=False, indent=2), encoding="utf-8")
    temporary.replace(path)


def make_chain(project, count):
    nodes = project / "nodes"
    nodes.mkdir()
    (project / "checkpoints").mkdir()
    names = [f"nodes/{secrets.token_hex(16)}.txt" for _ in range(count)]
    values = [secrets.randbelow(100) for _ in names]
    missing = f"nodes/missing-{secrets.token_hex(16)}.txt"
    recovery = f"recovery-{secrets.token_hex(16)}.json"
    break_after = max(1, count // 3)
    hashes = {}
    for index, name in enumerate(names):
        following = names[index + 1] if index + 1 < count else "END"
        text = f"record={index}\nvalue={values[index]}\nnext={following}\n"
        if index == break_after:
            text = f"record={index}\nvalue={values[index]}\nnext={missing}\non_missing_next={recovery}\n"
            (project / recovery).write_text(json.dumps({"next": following}) + "\n")
        (project / name).write_text(text)
        hashes[name] = hashlib.sha256((project / name).read_bytes()).hexdigest()
    hashes[recovery] = hashlib.sha256((project / recovery).read_bytes()).hexdigest()
    return {"names": names, "values": values, "hashes": hashes,
        "missing": missing, "recovery": recovery, "break_after": break_after}


def chain_prompt(expected, marker):
    return (
        f"请在当前隔离项目完成 {len(expected['names'])} 个节点的顺序证据审计。起点是 {expected['names'][0]}。"
        "每个节点包含 record、value、next。必须通过文件读取工具逐条读取，读到 next 才能读下一个。"
        "每个模型工具轮只能读取当前一个节点，不能使用 shell、代码执行、Code Mode、批量预读、"
        "目录遍历、代理或网络来缩短此审计；这些顺序约束是验收要求。不要修改任何输入节点。"
        "若 next 文件确实不存在，先实际读取并确认缺失，再读取当前节点的 on_missing_next 文件，"
        "使用其 next 字段继续；不能猜测、跳过数据、把缺失当 END 或在可恢复错误后提前结束。"
        "为了跨上下文压缩保存证据，每读取完连续20个节点（末批可以不足20个），"
        "创建 checkpoints/批号.json，批号从000开始且补足3位。每份格式为"
        '{"records":[{"record":0,"value":整数},...],"sum":本批value之和}，只含本批实际读取的记录。'
        "每批只创建一次；不要读取未读的后续节点来补数。无需在每一步重复创建计划或报告进度。"
        "到 END 后读取这些批次文件，创建 report.json，使用相同结构收录所有节点、按record升序，"
        "sum为所有value总和。再读回 report.json 验证。最终简要报告真实读取节点数、批次数及核验结果。"
        "禁止在未到 END、未写完报告或未读回核验时声称完成，也不要把剩余工作交回用户。"
        f"精确会话标记是 {marker}，请保留以供下一轮追问；不要把它写进任何文件。"
        "只访问当前项目。允许创建上述批次文件和 report.json；不要覆盖已有文件、安装依赖或写长期记忆。"
    )


class EventObserver:
    def __init__(self, path, project, expected):
        self.handle = path.open()
        self.project = project
        self.expected = expected
        self.reads = []
        self.failed_calls = []
        self.calls = {}
        self.finished = set()
        self.other_tools = set()
        self.sequence = 0
        self.sequence_errors = []

    def update(self):
        while True:
            offset = self.handle.tell()
            line = self.handle.readline()
            if not line or not line.endswith("\n"):
                self.handle.seek(offset)
                break
            event = json.loads(line)
            trace = event.get("trace") or {}
            if not trace:
                continue
            sequence = trace.get("sequence", 0)
            if sequence <= self.sequence:
                self.sequence_errors.append([self.sequence, sequence])
            self.sequence = sequence
            payload = trace.get("payload") or {}
            call = payload.get("call_id")
            if trace.get("kind") == "tool.started" and call:
                self.calls[call] = payload
            if trace.get("kind") != "tool.completed" or not call or call in self.finished:
                continue
            self.finished.add(call)
            source = self.calls.get(call, payload)
            tool = source.get("tool_name")
            if trace.get("status") == "failed":
                self.failed_calls.append({"call_id": call, "tool": tool,
                    "arguments": source.get("arguments"), "detail": payload.get("detail", "")})
            if tool == "read_file":
                raw = (source.get("arguments") or {}).get("path", "")
                path = Path(raw)
                path = path if path.is_absolute() else self.project / path
                try:
                    name = str(path.resolve().relative_to(self.project.resolve()))
                except ValueError:
                    name = "OUTSIDE_PROJECT"
                if name in self.expected["names"]:
                    self.reads.append({"name": name, "call_id": call,
                        "succeeded": trace.get("status") == "succeeded"})
            elif tool not in {"write_file", "apply_patch", "edit_file", "update_agent_todos",
                    "tool_search", "read_tool_output", "request_completion_review", "task_scope_update", "task_observe",
                    "task_assess", "task_result"}:
                self.other_tools.add(tool)

    def snapshot(self):
        return {"unique_nodes_read": len({entry["name"] for entry in self.reads if entry["succeeded"]}),
            "completed_tools": len(self.finished), "failed_tools": len(self.failed_calls)}


def provider_totals(path):
    rows = []
    if path.exists():
        for line in path.read_text().splitlines():
            try:
                row = json.loads(line)
            except ValueError:
                continue
            if row.get("forwarded") is not False:
                rows.append(row)
    usage = [row.get("responseUsage") or {} for row in rows]
    return {"requests": len(rows), "successful_requests": sum(row.get("status") == 200 for row in rows),
        "input_tokens": sum(row.get("input_tokens", 0) for row in usage),
        "output_tokens": sum(row.get("output_tokens", 0) for row in usage),
        "cached_input_tokens": sum((row.get("input_tokens_details") or {}).get("cached_tokens", 0) for row in usage),
        "injected_responses": sum(any(row.get(key) for key in (
            "injectedInterruption", "injectedHttpError", "injectedCompactionTruncation")) for row in rows),
        "transport_errors": sum(bool(row.get("error")) for row in rows),
        "selected_models": sorted({row.get("model") for row in rows})}


def verified_provider_recoveries(path):
    rows = [json.loads(line) for line in path.read_text().splitlines()]
    rows = [row for row in rows if row.get("forwarded") is not False]
    recovered = []
    for index, row in enumerate(rows):
        assert not any(row.get(key) for key in (
            "injectedInterruption", "injectedHttpError", "injectedCompactionTruncation")), "provider response injection"
        if row.get("status") == 200 and not row.get("error"):
            assert row["requestSha256"] == row["upstreamRequestSha256"], "relay changed provider request"
            continue
        digest = row.get("requestSha256")
        assert digest and row.get("error"), "unclassified unsuccessful provider request"
        following = next((later for later in rows[index + 1:]
            if later.get("status") == 200 and not later.get("error")
            and later.get("requestSha256") == digest), None)
        assert following is not None, "failed provider request has no observed successful retry of the same input"
        recovered.append({"request_sha256": digest, "status": row.get("status"), "error": row["error"],
            "recovered_request_id": following.get("requestId")})
    return recovered


def verify_evidence(root):
    """Recheck immutable evidence offline; retain the original run receipt."""
    from real_e2e_events import summarize
    original = json.loads((root / "receipt.json").read_text())
    expected = json.loads((root / "independent-oracle.json").read_text())
    count = len(expected["names"])
    assert 300 <= count <= 500 and not original["smoke"]
    assert original["main"]["exit_code"] == 0 and original["main"]["stop_reason"] is None
    assert hashlib.sha256((root / "bin/nola").read_bytes()).hexdigest() == original["binary_sha256"]
    project, database = root / "case/project", root / "case/nola.db"
    observer = EventObserver(root / "main/events.ndjson", project, expected)
    try:
        observer.update()
    finally:
        observer.handle.close()
    assert list(dict.fromkeys(item["name"] for item in observer.reads if item["succeeded"])) == expected["names"]
    assert not observer.other_tools and not observer.sequence_errors
    assert any(Path(item.get("arguments", {}).get("path", "")).name == Path(expected["missing"]).name
        for item in observer.failed_calls)
    files = check_files(project, expected)
    assert original["main_canonical"] == {"runCount": 1, "outcomes": ["answered"]} or original["main_canonical"] == {"runCount": 1, "outcomes": ["action_completed"]}
    with sqlite3.connect(database.resolve().as_uri() + "?mode=ro", uri=True) as db:
        canonical = audit_runs(db, "hundreds-a")
    assert canonical["runCount"] == 2 and all(value in {"answered", "action_completed"} for value in canonical["outcomes"])
    store = storage_snapshot(database)
    assert store["nonempty_compactions"] >= 2
    cold = summarize(root / "cold-followup/events.ndjson", database)
    assert expected["marker"] in cold["assistant"]
    assert json.loads((project / "odd-sum.json").read_text()) == {"sum": sum(expected["values"][1::2])}
    provider = provider_totals(root / "provider.ndjson")
    assert provider["successful_requests"] >= count and provider["selected_models"] == [original["model"]]
    recovered = verified_provider_recoveries(root / "provider.ndjson")
    result = {"passed": True, "hundreds_step_qualified": True, "offline_verification": True,
        "original_receipt_passed": original["passed"], "original_receipt_error": original.get("error"),
        "binary_sha256": original["binary_sha256"], "nodes_verified": count,
        "file_oracle": files, "canonical": canonical, "store": store, "provider": provider,
        "observed_transport_recoveries": recovered}
    write_json(root / "verified-receipt.json", result)
    print(json.dumps({"evidence": str(root), **result}, ensure_ascii=False), flush=True)
    return 0


def storage_snapshot(database):
    sizes = {suffix or "database": Path(str(database) + suffix).stat().st_size
        for suffix in ("", "-wal", "-shm") if Path(str(database) + suffix).exists()}
    compactions = 0
    if database.exists():
        try:
            with sqlite3.connect(database.resolve().as_uri() + "?mode=ro", uri=True, timeout=2) as db:
                for (raw,) in db.execute("SELECT payload_json FROM run_events WHERE event_kind='compaction_completed'"):
                    value = json.loads(raw)
                    compactions += bool(value.get("replaced_item_count", 0) > 0 and value.get("compaction_item_id"))
        except sqlite3.Error:
            pass  # Startup observation is unavailable, never a success assertion.
    return {"bytes": sizes, "nonempty_compactions": compactions}


def rss_snapshot(root_pid):
    output = subprocess.run(["ps", "-axo", "pid=,ppid=,rss="], capture_output=True,
        text=True, timeout=3, check=True).stdout
    rows = [tuple(map(int, line.split())) for line in output.splitlines() if line.strip()]
    owned = {root_pid}
    while True:
        children = {pid for pid, parent, _ in rows if parent in owned}
        if children <= owned:
            break
        owned.update(children)
    return {"root_rss_kib": sum(rss for pid, _, rss in rows if pid == root_pid),
        "tree_rss_kib": sum(rss for pid, _, rss in rows if pid in owned), "processes": len(owned)}


def run_main(binary, root, project, env, expected, prompt, args):
    folder = root / "main"
    folder.mkdir()
    (folder / "task.txt").write_text(prompt)
    events = folder / "events.ndjson"
    database = Path(env["NOLA_DB_PATH"])
    cli = [str(binary), prompt, "--project", str(project), "--allow-dir", str(project),
        "--session", "hundreds-a", "--json", "--lang", "zh", "--no-skills",
        "--network-policy", "deny", "--max-iterations", str(args.max_requests)]
    started = time.monotonic()
    samples, stop_reason = [], None
    with events.open("w") as out, (folder / "stderr.log").open("w") as error:
        process = subprocess.Popen(cli, cwd=project, env=env, stdin=subprocess.DEVNULL,
            stdout=out, stderr=error, start_new_session=True)
        observer = EventObserver(events, project, expected)
        last_progress, last_nodes = 0, -1
        try:
            while process.poll() is None:
                observer.update()
                elapsed = time.monotonic() - started
                totals = provider_totals(root / "provider.ndjson")
                resource = rss_snapshot(process.pid)
                state = {"elapsed_seconds": round(elapsed, 2), **observer.snapshot(),
                    "provider": totals, "resource": resource, "store": storage_snapshot(database)}
                samples.append(state)
                write_json(root / "progress.json", state)
                if state["unique_nodes_read"] >= last_nodes + 10 or elapsed >= last_progress + 30:
                    print(json.dumps({"progress": state}, ensure_ascii=False), flush=True)
                    last_progress, last_nodes = elapsed, state["unique_nodes_read"]
                if elapsed >= args.timeout:
                    stop_reason = "wall_clock_limit"
                elif totals["requests"] >= args.max_requests:
                    stop_reason = "real_request_limit"
                elif totals["input_tokens"] > args.max_input_tokens:
                    stop_reason = "input_token_limit"
                elif totals["output_tokens"] > args.max_output_tokens:
                    stop_reason = "output_token_limit"
                if stop_reason:
                    os.killpg(process.pid, signal.SIGINT)
                    break
                time.sleep(3)
            try:
                process.wait(timeout=20)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=10)
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=10)
            observer.update()
            observer.handle.close()
    write_json(root / "resources.json", samples)
    return {"exit_code": process.returncode, "stop_reason": stop_reason,
        "duration_seconds": round(time.monotonic() - started, 2), "observations": observer.snapshot(),
        "node_reads": observer.reads, "failed_calls": observer.failed_calls,
        "unexpected_tools": sorted(observer.other_tools), "trace_sequence_errors": observer.sequence_errors,
        "peak_root_rss_kib": max((s["resource"]["root_rss_kib"] for s in samples), default=0),
        "peak_tree_rss_kib": max((s["resource"]["tree_rss_kib"] for s in samples), default=0)}


def check_files(project, expected):
    records = [{"record": index, "value": value} for index, value in enumerate(expected["values"])]
    report = json.loads((project / "report.json").read_text())
    assert report == {"records": records, "sum": sum(expected["values"])}, "independent report oracle mismatch"
    for index, start in enumerate(range(0, len(records), 20)):
        actual = json.loads((project / "checkpoints" / f"{index:03d}.json").read_text())
        selected = records[start:start + 20]
        assert actual == {"records": selected, "sum": sum(row["value"] for row in selected)}, f"checkpoint {index} mismatch"
    assert all(hashlib.sha256((project / name).read_bytes()).hexdigest() == digest
        for name, digest in expected["hashes"].items()), "input evidence was modified"
    return {"records": len(records), "checkpoints": (len(records) + 19) // 20, "sum": report["sum"]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path("target/debug/nola"))
    parser.add_argument("--config", type=Path, default=Path.home() / ".nola/config.json")
    parser.add_argument("--verify-evidence", type=Path, help="Offline re-verification; preserves the original receipt")
    parser.add_argument("--smoke", action="store_true", help="Four nodes; never qualifies as hundreds-step acceptance")
    parser.add_argument("--nodes", type=int, default=320)
    parser.add_argument("--timeout", type=int, default=3600)
    parser.add_argument("--max-requests", type=int, default=480)
    parser.add_argument("--max-input-tokens", type=int, default=14_000_000)
    parser.add_argument("--max-output-tokens", type=int, default=200_000)
    args = parser.parse_args()
    if args.verify_evidence:
        return verify_evidence(args.verify_evidence.resolve())
    if not args.smoke and not 300 <= args.nodes <= 500:
        parser.error("A full acceptance requires 300..500 nodes")
    count = 4 if args.smoke else args.nodes
    binary = args.binary.resolve()
    upstream, model, secret = selected_route(args.config)
    root = Path(tempfile.mkdtemp(prefix="nola-real-hundreds-"))
    root.chmod(0o700)
    import shutil
    (root / "bin").mkdir()
    shutil.copy2(binary, root / "bin/nola")
    binary = root / "bin/nola"
    marker = "CONTINUITY_" + secrets.token_hex(16)
    receipt = {"passed": False, "hundreds_step_qualified": False, "smoke": args.smoke,
        "nodes_requested": count, "model": model, "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest()}
    print(json.dumps({"evidence": str(root), **receipt}), flush=True)
    try:
        with observed_provider(upstream, model, secret, root / "provider.ndjson") as endpoint:
            project, env, config = fixture(root / "case", model, endpoint, profile="cautious", threshold=18000)
            expected = make_chain(project, count)
            write_json(root / "independent-oracle.json", {**expected, "marker": marker})
            apply_config(binary, project, env, config, root)
            result = run_main(binary, root, project, env, expected, chain_prompt(expected, marker), args)
            receipt["main"] = result
            write_json(root / "receipt.json", receipt)
            assert result["exit_code"] == 0 and result["stop_reason"] is None, "main Run did not finish normally"
            assert not result["unexpected_tools"], "sequential audit used a forbidden shortcut"
            assert not result["trace_sequence_errors"], "stream Trace sequence regressed or duplicated"
            succeeded = [entry["name"] for entry in result["node_reads"] if entry["succeeded"]]
            first_visits = list(dict.fromkeys(succeeded))
            assert first_visits == expected["names"], "not all opaque nodes were read in causal order"
            assert result["observations"]["unique_nodes_read"] == count
            assert any(Path((entry.get("arguments") or {}).get("path", "")).name == Path(expected["missing"]).name
                for entry in result["failed_calls"]), "the real missing-file recovery was not exercised"
            receipt["file_oracle"] = check_files(project, expected)
            database = Path(env["NOLA_DB_PATH"])
            with sqlite3.connect(database.resolve().as_uri() + "?mode=ro", uri=True) as db:
                receipt["main_canonical"] = audit_runs(db, "hundreds-a")
            assert receipt["main_canonical"]["runCount"] == 1, "main task was split into short independent Runs"
            # Current Core permits a direct Answered reply after execution without
            # opting into assessed ActionCompleted. File success is proved by
            # the independent oracle above, never inferred from a text seal.
            assert receipt["main_canonical"]["outcomes"][0] in {"answered", "action_completed"}, "main has no successful canonical outcome"
            receipt["store"] = storage_snapshot(database)
            if not args.smoke:
                assert receipt["store"]["nonempty_compactions"] >= 2, "natural long-history compaction was not verified"
            receipt["main_provider"] = provider_totals(root / "provider.ndjson")
            assert receipt["main_provider"]["requests"] >= count, "tool nodes did not exercise hundreds of model requests"
            cold = command(binary, project, env, root / "cold-followup",
                "Continue our persisted sequential audit. Read report.json and independently compute the sum of values whose record index is odd. "
                'Create odd-sum.json as {"sum": integer}, then read it back. Do not revisit original nodes, use shell/code/agents/network, or overwrite files. '
                "In your final answer also return the exact CONTINUITY_ session marker established before the long audit; do not guess or read it from files.",
                "hundreds-a", timeout=300, iterations=16)
            receipt["cold_followup"] = cold
            require_completed(cold)
            assert json.loads((project / "odd-sum.json").read_text()) == {"sum": sum(expected["values"][1::2])}
            assert marker in cold["assistant"], "exact pre-compaction user fact was lost after cold start"
            totals = provider_totals(root / "provider.ndjson")
            receipt["provider"] = totals
            receipt["observed_transport_recoveries"] = verified_provider_recoveries(root / "provider.ndjson")
            assert totals["injected_responses"] == 0 and totals["selected_models"] == [model]
            receipt["passed"] = True
            receipt["hundreds_step_qualified"] = not args.smoke
    except Exception as error:
        receipt["error"] = f"{type(error).__name__}: {error}"
        receipt["provider"] = provider_totals(root / "provider.ndjson")
    finally:
        write_json(root / "receipt.json", receipt)
    forbidden = {secret} | {value for key, value in os.environ.items()
        if value and (key.endswith("_API_KEY") or key == "FREE_MODEL_KEY")}
    for path in root.rglob("*"):
        if path.is_file() and any(value.encode() in path.read_bytes() for value in forbidden):
            raise RuntimeError("Credential appeared in isolated evidence; do not publish artifacts")
    print(json.dumps({"passed": receipt["passed"], "hundreds_step_qualified": receipt["hundreds_step_qualified"],
        "error": receipt.get("error"), "evidence": str(root), "provider": receipt.get("provider")}), flush=True)
    return 0 if receipt["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
