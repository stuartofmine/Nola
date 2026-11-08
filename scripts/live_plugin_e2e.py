#!/usr/bin/env python3
"""Opt-in, isolated plugin/Skill/credential/MCP acceptance against a LAN model.

Never opens the user's Application database. Credentials are read from the
existing CLI configuration, copied only to a mode-0600 temporary configuration,
and never printed. Test artifacts are retained for trajectory inspection.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile
import uuid
from urllib.parse import urlsplit


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--config", type=Path, default=Path.home() / ".nola/config.json")
    args = parser.parse_args()
    if os.environ.get("NOLA_RUN_LIVE_PLUGIN_E2E") != "1":
        raise SystemExit("Set NOLA_RUN_LIVE_PLUGIN_E2E=1 to run real model requests")
    llm = json.loads(args.config.read_text())["llm"]
    if not (urlsplit(llm["endpoint"]).hostname or "").startswith("192.168."):
        raise SystemExit("This acceptance requires the configured 192.168.* model service")
    root = Path(tempfile.mkdtemp(prefix="nola-plugin-e2e-"))
    home, package, workspace = (root / name for name in ("home", "package", "workspace"))
    for directory in (home / ".nola", package / "audit", workspace):
        directory.mkdir(parents=True)
    config = home / ".nola/config.json"
    config.touch(mode=0o600)
    config.write_text(json.dumps({"llm": llm, "network_policy": "allow"}))
    environment = dict(os.environ, NOLA_HOME=str(home), NOLA_DB_PATH=str(root / "application.db"),
                       NOLA_APPLICATION_SERVICE_EXECUTABLE=str(args.binary.resolve()))
    environment.pop("NOLA_HOST_CAPABILITY_MANIFEST", None)
    marker = "SHIPMENT-" + uuid.uuid4().hex
    audit = root / "mcp-calls.jsonl"
    script = package / "server.py"
    script.write_text('''#!/usr/bin/env python3
import json, os, sys
assert os.environ.get('FIXTURE_TOKEN') == 'integration-only'
for line in sys.stdin:
    r = json.loads(line)
    if 'id' not in r: continue
    method = r.get('method')
    if method == 'initialize':
        result = {'protocolVersion':'2025-06-18','capabilities':{}}
    elif method == 'tools/list':
        result = {'tools':[{'name':'shipment_status','description':'Query authoritative shipment status by tracking ID.',
            'annotations':{'readOnlyHint':True},'inputSchema':{'type':'object','properties':{'tracking_id':{'type':'string'}},'required':['tracking_id'],'additionalProperties':False}}]}
    elif method == 'tools/call':
        with open(sys.argv[1], 'a') as f: f.write(json.dumps(r['params'])+'\\n')
        result = {'content':[{'type':'text','text':json.dumps({'tracking_id':r['params']['arguments']['tracking_id'],'status':'delivered','receipt':sys.argv[2]+'-'+r['params']['arguments']['tracking_id']})}]}
    else:
        print(json.dumps({'jsonrpc':'2.0','id':r['id'],'error':{'code':-32601,'message':'unsupported'}}), flush=True)
        continue
    print(json.dumps({'jsonrpc':'2.0','id':r['id'],'result':result}), flush=True)
''')
    script.chmod(0o700)
    (package / "audit/SKILL.md").write_text("---\nname: shipment-audit\ndescription: Verify parcel delivery using the shipment status service and report its receipt.\n---\n\nDiscover the shipment_status tool and query the requested tracking ID. Report the returned status and exact receipt. Never invent a status or receipt.\n")
    (package / "audit/NOLA_MANIFEST.json").write_text(json.dumps({
        "version": "1", "required_tools": [{"provider": "mcp", "server_id": "plugin:shipment-fixture:mcp:shipment", "tool_name": "shipment_status"}]}))
    (package / "nola-plugin.json").write_text(json.dumps({
        "schema_version": 1, "id": "shipment-fixture", "version": "1", "display_name": "Shipment acceptance", "description": "Isolated test only",
        "skills": [{"id": "audit", "path": "./audit"}],
        "mcp_servers": [{"id": "shipment", "transport": {"type": "stdio", "executable": {"type": "package", "path": "./server.py"},
                         "args": [str(audit), marker], "secret_env": {"FIXTURE_TOKEN": "fixture-token"}}}]}))
    def run(label, command, stdin=None):
        result = subprocess.run([str(args.binary.resolve()), *command], input=stdin, text=True,
                                capture_output=True, cwd=workspace, env=environment, timeout=240)
        (root / (label + ".stdout")).write_text(result.stdout)
        (root / (label + ".stderr")).write_text(result.stderr)
        if result.returncode:
            raise RuntimeError(f"{label} failed ({result.returncode}); inspect {root}")
        return result.stdout
    def traces(output):
        return [event["trace"] for line in output.splitlines()
                if line.startswith("{") and (event := json.loads(line)).get("type") == "agent_trace"]
    def require_completed(output):
        if not any(event["kind"] == "run.completed" and event["payload"].get("reason") in
                   {"action_completed", "answered"} for event in traces(output)):
            raise RuntimeError(f"task did not complete successfully; inspect {root}")
    def require_receipt(output, tracking_id, start=0):
        calls = [json.loads(line) for line in audit.read_text().splitlines()][start:] if audit.exists() else []
        receipt = marker + "-" + tracking_id
        answers = [event["payload"].get("arguments", {}).get("answer", "")
                   for event in traces(output) if event["kind"] == "tool.completed"
                   and event["payload"].get("tool_name") == "requirements_update"]
        if not any(call.get("arguments", {}).get("tracking_id") == tracking_id for call in calls) or not any(receipt in answer for answer in answers):
            raise RuntimeError(f"missing current tracking ID call or final receipt; inspect {root}")
    try:
        run("install", ["plugin", "install", str(package)])
        run("credential", ["plugin", "credential-set", "shipment-fixture", "fixture-token"], "integration-only\n")
        run("enable", ["plugin", "enable", "shipment-fixture"])
        session = str(uuid.uuid4())
        flags = ["--project", str(workspace), "--allow-dir", str(workspace), "--yes", "--json", "--session", session]
        first = run("first-turn", ["请使用 shipment-audit 技能核实包裹 NOLA-42 的签收情况，给出服务返回的原始回执。", *flags])
        require_completed(first)
        skill_called = any(event["kind"] == "tool.completed" and event["payload"].get("tool_name") == "Skill"
                           and event["payload"].get("arguments", {}).get("skill") == "plugin:shipment-fixture:skill:audit"
                           for event in traces(first))
        if not audit.exists() or marker not in first or not skill_called:
            raise RuntimeError(f"missing Skill/MCP/receipt evidence; inspect {root}")
        require_receipt(first, "NOLA-42")
        before = len(audit.read_text().splitlines())
        second = run("cold-followup", ["再帮我查一下 NOLA-43，别沿用刚才的结果。", *flags])
        require_completed(second)
        require_receipt(second, "NOLA-43", before)
        if len(audit.read_text().splitlines()) <= before or marker not in second:
            raise RuntimeError(f"cold follow-up did not invoke MCP; inspect {root}")
        before = len(audit.read_text().splitlines())
        third = run("english-followup", ["What is the current status of NOLA-45? Please use the same source and give its receipt.", *flags])
        require_completed(third)
        require_receipt(third, "NOLA-45", before)
        if len(audit.read_text().splitlines()) <= before or marker not in third:
            raise RuntimeError(f"English follow-up did not invoke MCP; inspect {root}")
        run("disable", ["plugin", "disable", "shipment-fixture"])
        before = audit.read_text()
        disabled = run("disabled-turn", ["再查一下 NOLA-44；如果服务未启用请明确说明，不要换别的工具。", *flags])
        if not any(event["kind"] == "run.completed" for event in traces(disabled)):
            raise RuntimeError(f"disabled turn did not settle; inspect {root}")
        if audit.read_text() != before:
            raise RuntimeError(f"disabled plugin was called; inspect {root}")
        print(json.dumps({"passed": True, "evidence": str(root)}, ensure_ascii=False))
    finally:
        # Only the fixture's credential-bearing config is removed. Preserve
        # transcripts, package, and isolated DB for evidence; never user data.
        config.unlink(missing_ok=True)


if __name__ == "__main__":
    main()
