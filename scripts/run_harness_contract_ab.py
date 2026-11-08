#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Paired real-model smoke test; public checks, not a Terminal-Bench score.

Both arms use the same binary/configuration/fixture and a fresh isolated DB.
Only the shell-purpose schema mode changes. A single pair is diagnostic, not
statistical evidence for promoting either mode to the product default.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]


def metrics(log: Path) -> dict:
    result = {"tool_completions":0, "tool_failures":0, "validation_declarations":0,
              "protocol_argument_failures":0, "observed_core_result_claims":[]}
    for line in log.read_text(errors="replace").splitlines():
        try:
            event = json.loads(line)
        except ValueError:
            continue
        if not isinstance(event, dict):
            continue
        trace = event.get("trace") or {}
        if not isinstance(trace, dict):
            continue
        if trace.get("kind") == "run.completed":
            result["observed_core_result_claims"].append((trace.get("payload") or {}).get("reason"))
        if trace.get("kind") != "tool.completed":
            continue
        result["tool_completions"] += 1
        payload = trace.get("payload") or {}
        output = payload.get("result") or {}
        if trace.get("status") == "failed":
            result["tool_failures"] += 1
        arguments = payload.get("arguments") or payload.get("input") or {}
        if isinstance(arguments, dict) and arguments.get("operation_kind") == "validate":
            result["validation_declarations"] += 1
        # Diagnostic count only; never feeds routing, permissions or success.
        if "invalid_tool_arguments" in json.dumps(output):
            result["protocol_argument_failures"] += 1
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--config", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--seconds", type=float, default=600)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    source = json.loads(args.config.read_text())
    configuration = {"llm":source["llm"], "max_iterations":0, "network_policy":"allow"}
    if "model_capabilities" in source:
        configuration["model_capabilities"] = source["model_capabilities"]
    results = {"experiment":"shell-purpose-public-smoke", "official_benchmark":False,
               "model":source["llm"]["model"], "seconds_per_arm":args.seconds,
               "binary_sha256":hashlib.sha256(args.binary.read_bytes()).hexdigest(), "arms":{}}
    for mode in ("required", "optional"):
        arm = args.output.resolve() / mode
        project = arm / "project"
        project.mkdir(parents=True)
        (project / ".git").mkdir()  # Isolate ancestor instruction discovery.
        (project / "ledger.py").write_text(
            "def fold(events):\n    state = {}\n    for op, key, value in events:\n"
            "        if op == 'set': state[key] = value\n"
            "        elif op == 'delete': state[key] = None\n    return state\n")
        checker_code = ("from ledger import fold; "
            "assert fold([])=={}; "
            "assert fold([('set','a',1),('set','a',2)])=={'a':2}; "
            "assert fold([('set','a',1),('delete','a',None)])=={}; "
            "assert fold([('delete','missing',None)])=={}; "
            "assert fold([('set','a',1),('set','b',2),('delete','a',None)])=={'b':2}; "
            "assert fold([('set','a',1),('delete','a',None),('set','a',3)])=={'a':3}")
        checker = {"argv":[sys.executable, "-c", checker_code], "cwd":str(project),
                   "timeout_seconds":10, "public":True}
        contract_path = arm / "public-check.json"
        contract_path.write_text(json.dumps(checker))
        config_path = project / "nola.json"
        config_path.write_text(json.dumps({**configuration, "allowed_dirs":[str(project)]}))
        config_path.chmod(0o600)
        environment = {**os.environ, "NOLA_DB_PATH":str(arm / "state.db"),
                       "NOLA_SKIP_ONBOARDING":"1", "NOLA_SHELL_PURPOSE_CONTRACT":mode}
        command = [sys.executable, str(ROOT / "evals/nola_agent_supervisor.py"), "--seconds", str(args.seconds), "--",
                   sys.executable, str(ROOT / "evals/nola_task_acceptance.py"),
                   "--contract", str(contract_path), "--report", str(arm / "acceptance.json"),
                   "--seconds", str(args.seconds), "--", str(args.binary.resolve()),
                   "--project", str(project), "--session", "contract-smoke", "--json", "--yes",
                   "--max-iterations", "0"]
        started = time.monotonic()
        try:
            with (arm / "events.ndjson").open("w") as log:
                process = subprocess.run(command, cwd=project, env=environment, text=True,
                    input="修复 ledger.py 的 fold：set 覆盖键，delete 删除键，删除不存在的键是无操作，不能影响其他键。添加并运行有断言的回归测试。仅修改当前项目。",
                    stdout=log, stderr=subprocess.STDOUT, timeout=args.seconds+15)
        finally:
            config_path.unlink(missing_ok=True)
        report_path = arm / "acceptance.json"
        results["arms"][mode] = {"elapsed_seconds":time.monotonic()-started,
            "harness_exit_code":process.returncode, **metrics(arm / "events.ndjson"),
            "acceptance":json.loads(report_path.read_text()) if report_path.exists() else None}
        (args.output / "summary.json").write_text(json.dumps(results, ensure_ascii=False, indent=2)+"\n")
        print(f"{mode}: exit={process.returncode}, elapsed={results['arms'][mode]['elapsed_seconds']:.1f}s", flush=True)


if __name__ == "__main__":
    main()
