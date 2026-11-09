#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Real selected-route long-task acceptance using current configuration only.

Runs in disposable Stores/projects. Credentials stay in the relay's memory;
model processes receive neither the key nor the user's configuration. A
transparent relay records transport counters, never substitutes model output.
"""
from __future__ import annotations
import argparse
import contextlib
import hashlib
from http.server import ThreadingHTTPServer
import json
import os
from pathlib import Path
import signal
import sqlite3
import subprocess
import sys
import tempfile
import threading
import time
import uuid
from urllib.parse import urlsplit

from canonical_run_audit import audit_runs
from real_e2e_events import summarize
from provider_stream_acceptance import require_text_stream
from e2e.provider_probe import Handler, Probe
from e2e.real_overwrite_permission import isolated_environment


def selected_route(path):
    config = json.loads(path.read_text())
    llm = config["llm"]
    selected = llm["default_model"]
    connection = next(c for c in llm["connections"] if c["id"] == selected["connection"])
    model = next(m for m in connection["models"] if m["id"] == selected["model"])
    if connection["protocol"] != "responses":
        raise ValueError("This acceptance matrix requires the selected Responses route")
    url = urlsplit(connection["endpoint"])
    if url.scheme != "https" or url.username or url.password or url.query or url.fragment:
        raise ValueError("Expected a credential-free HTTPS provider URL")
    secret = os.environ.get(connection.get("api_key_env", ""), "")
    if not secret or secret.strip() != secret or "\r" in secret or "\n" in secret:
        raise ValueError("Selected route credential is unavailable")
    return connection["endpoint"], model["id"], secret


@contextlib.contextmanager
def observed_provider(upstream, model, secret, log, *, listen_port=0, request_observer=None):
    expected_path = urlsplit(upstream).path.rstrip("/") + "/responses"
    class RestrictedProbe(Probe):
        def response_override(self, path, request, headers):
            if request_observer is not None:
                request_observer(request)
            if path != expected_path or request.get("model") != model or not request.get("input"):
                return 403, b'{"error":{"message":"Acceptance relay only permits its selected generation route"}}'
            return None
    class AuthorizedHandler(Handler):
        def do_POST(self):
            if "Authorization" in self.headers:
                del self.headers["Authorization"]
            self.headers["Authorization"] = "Bearer " + secret
            super().do_POST()
    server = ThreadingHTTPServer(("127.0.0.1", listen_port), AuthorizedHandler)
    server.probe = RestrictedProbe(upstream, log, 180)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"http://127.0.0.1:{server.server_port}{urlsplit(upstream).path.rstrip('/')}"
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)


def fixture(root, model, endpoint, profile="allow_all", threshold=48000):
    project = root / "project"
    project.mkdir(parents=True)
    env = isolated_environment(root, profile)
    allowed = {"PATH", "HOME", "USER", "LOGNAME", "SHELL", "TMPDIR", "LANG", "LC_ALL", "LC_CTYPE", "TERM", "TZ"}
    env = {key:value for key,value in env.items() if key in allowed or key.startswith("NOLA_")}
    env.update(NOLA_AUTOMATIC_MEMORY_MAINTENANCE_ENABLED="0", NOLA_RESIDENT_MEMORY_RECALL_ENABLED="0")
    # Explicit fixture assertions, not model-name-based capability discovery.
    declaration = {"toolCalls": True, "imageInput": False, "reasoning": True, "samplingParameters": False}
    config = {"llm": {"connections": [{"id": "acceptance", "name": "Owned acceptance relay",
        "endpoint": endpoint, "protocol": "responses", "models": [{"id": model, "name": model,
        "context_window": 65536, "output_limit": 8192, "capabilities": declaration}]}],
        "default_model": {"connection": "acceptance", "model": model}},
        "compaction": {"context_window_tokens": 65536, "token_threshold": threshold, "keep_recent_messages": 2}}
    config_path = root / "home/.nola/config.json"
    config_path.parent.mkdir(parents=True, exist_ok=True)
    config_path.write_text(json.dumps(config))
    return project, env, config_path


def command(binary, project, env, folder, prompt, session, timeout=600, iterations=32, allow_pending=False, no_tools=False):
    folder.mkdir(parents=True)
    (folder / "task.txt").write_text(prompt)
    args = [str(binary), prompt, "--project", str(project), "--allow-dir", str(project),
        "--session", session, "--json", "--lang", "zh", "--no-skills", "--network-policy", "deny",
        "--max-iterations", str(iterations)]
    if no_tools:
        args.append("--no-tools")
    started = time.monotonic()
    with (folder / "events.ndjson").open("w") as out, (folder / "stderr.log").open("w") as err:
        process = subprocess.Popen(args, cwd=project, env=env, stdout=out, stderr=err, start_new_session=True)
        timed_out = False
        try:
            code = process.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            timed_out = True
            os.killpg(process.pid, signal.SIGINT)
            try: code = process.wait(timeout=20)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                code = process.wait(timeout=10)
    result = {"exit_code": code, "timed_out": timed_out, "duration_seconds": round(time.monotonic()-started, 2)}
    try:
        summary = summarize(folder / "events.ndjson", Path(env["NOLA_DB_PATH"]))
        (folder / "summary.json").write_text(json.dumps(summary, ensure_ascii=False, indent=2))
        result.update(tools=summary["toolsCompleted"], tool_count=len(summary["toolCallsCompleted"]),
            event_count=summary["eventCount"], assistant=summary["assistant"])
        with sqlite3.connect(Path(env["NOLA_DB_PATH"]).resolve().as_uri()+"?mode=ro", uri=True) as db:
            pending = db.execute("SELECT id,status FROM runs WHERE conversation_id=? AND status<>'sealed'", (session,)).fetchall()
            if allow_pending and pending:
                result["durable_waits"] = pending
            else:
                result["canonical"] = audit_runs(db, session)
    except Exception as error:
        result["audit_error"] = str(error)
    (folder / "result.json").write_text(json.dumps(result, ensure_ascii=False, indent=2))
    print(json.dumps({"turn": folder.name, **{k:v for k,v in result.items() if k not in {"assistant","tools"}}}), flush=True)
    return result


def require_completed(result):
    assert result["exit_code"] == 0 and not result["timed_out"] and not result.get("audit_error"), result
    assert all(k in {"answered", "action_completed"} for k in result["canonical"]["outcomes"]), result["canonical"]


def chain_case(binary, root, model, endpoint, threshold=48000, seed_history=False):
    project, env, config = fixture(root, model, endpoint, threshold=threshold)
    apply_config(binary, project, env, config, root)
    seeded = []
    fact = "CTX_FACT_" + uuid.uuid4().hex
    if seed_history:
        (root/"expected-fact.json").write_text(json.dumps({"fact":fact}))
        for index in range(2):
            logs = " ".join(f"log{index}-{number:05d}" for number in range(6000))
            prompt = (f"这是当前验收的长上下文压力数据。请保留后续要用的精确验收代码 {fact}。"
                "其余合成日志无需保留细节，不写文件、不调用工具，只回复 ACK。日志：" + logs)
            turn = command(binary,project,env,root/f"history-{index}",prompt,"chain-a",timeout=240,no_tools=True)
            require_completed(turn);seeded.append(turn)
    names = [uuid.uuid4().hex + ".txt" for _ in range(16)]
    values = [int(uuid.uuid4().hex[:5], 16) for _ in names]
    for i,name in enumerate(names):
        (project/name).write_text(f"record={i}\nvalue={values[i]}\nnext={names[i+1] if i+1<len(names) else 'END'}\n")
    original = {name:hashlib.sha256((project/name).read_bytes()).hexdigest() for name in names}
    prompt = (f"做一次顺序证据审计：从 {names[0]} 开始，按每个文件的 next 链接逐条读取到 END。"
        "本场景验证长工具会话，必须每次只用文件读取工具读取当前一个节点，看到 next 后再读取下一个；"
        "不要用 shell、程序、批量预读、代理或联网，也不要改节点。收集所有 record/value，最后只创建 report.json，"
        "格式为 {\"records\":[{\"record\":0,\"value\":整数},...],\"sum\":所有value之和}，"
        "再读回核验。完成后报告读取节点数与核验结果；缺步骤不能声称完成。")
    result = command(binary, project, env, root/"chain", prompt, "chain-a")
    require_completed(result)
    actual = json.loads((project/"report.json").read_text())
    assert actual == {"records":[{"record":i,"value":v} for i,v in enumerate(values)],"sum":sum(values)}
    assert all(hashlib.sha256((project/n).read_bytes()).hexdigest()==h for n,h in original.items())
    assert result["tool_count"] >= 18, "Long sequential tool loop was not exercised"
    followup = command(binary, project, env, root/"cold-followup",
        "Continue from our persisted audit. Read report.json, independently calculate the sum of the values whose record index is odd, and write odd-sum.json as {\"sum\": integer}. Do not reread the original chain, use shell, delegate, or access the network. Verify the new file.", "chain-a")
    require_completed(followup)
    assert json.loads((project/"odd-sum.json").read_text()) == {"sum":sum(values[1::2])}
    turns = seeded + [result,followup]
    if seed_history:
        recalled = command(binary,project,env,root/"compacted-recall",
            "Return only the exact acceptance code established in our earlier long-context messages. Do not guess, reread files, or use tools.",
            "chain-a", timeout=180, no_tools=True)
        require_completed(recalled)
        assert fact in recalled["assistant"], "The exact earlier fact was lost after compaction"
        turns.append(recalled)
    return {"passed":True,"nodes":len(names),"turns":turns}


def pipeline_case(binary, root, model, endpoint):
    project, env, config = fixture(root, model, endpoint)
    apply_config(binary, project, env, config, root)
    (project/"data").mkdir()
    rows=[]
    for shard in range(6):
        lines=["id,customer,amount,status"]
        for j in range(12):
            index=shard*12+j; amount=(index*137)%9991
            row=(f"ord-{index}",f"customer-{index%7}",f"{amount//100}.{amount%100:02d}","paid" if index%5 else "cancelled")
            rows.append(row); lines.append(",".join(row))
        (project/f"data/shard-{shard}.csv").write_text("\n".join(lines)+"\n")
    (project/"data/duplicate.csv").write_text("id,customer,amount,status\n"+",".join(rows[1])+"\n")
    (project/"data/bad.csv").write_text("id,customer,amount,status\nbad-1,customer-1,not-money,paid\n")
    (project/"README.md").write_text("Implement a stdlib-only Python reconciliation CLI. See the task for its complete contract.\n")
    prompt = ("在此隔离项目完成可运行的 Python 标准库订单对账工具 reconcile.py 和自己的回归测试。"
        "命令 python3 reconcile.py --input DIR --output FILE 扫描 DIR/*.csv。列为id,customer,amount,status。"
        "只累计 paid；金额非负且最多两位小数，用精确整数分计算。相同id且内容相同只计一次；"
        "相同id内容不同必须失败。任何非法金额/未知状态/缺列都失败且 exit=2；失败不能覆盖已有输出。"
        "成功原子写 JSON {\"customers\":{customer:总分},\"total_cents\":总分,\"paid_orders\":去重后的paid订单数}，"
        "客户键稳定排序。提供 README 使用方法。先用一次文件读取尝试读取 missing-spec.md，确认其不存在，"
        "不要编造它的内容，然后按本消息契约继续。data/bad.csv 是故意的坏数据，不能删改原始输入；"
        "你应创建独立的有效数据目录供成功示例验证，并验证失败不覆盖输出与重复执行幂等。"
        "可以用本机 python3 和文件工具；不要联网、安装依赖、调用其他模型/代理、写长期记忆、访问项目外文件。"
        "必须实际运行测试并修复失败，最后创建 verification.md 记录实际命令与结果，不以写出代码当完成。")
    result = command(binary, project, env, root/"pipeline", prompt, "pipeline-a", timeout=900)
    require_completed(result)
    checks = pipeline_oracle(project, root)
    assert all(check["passed"] for check in checks), "Independent pipeline oracle failed"
    assert (project/"verification.md").is_file()
    return {"passed":True,"held_out_checks":checks,"turns":[result]}


def pipeline_oracle(project, root):
    import csv
    import io
    audit = root / ("independent-oracle-" + uuid.uuid4().hex)
    audit.mkdir()
    env = {key:value for key,value in os.environ.items() if key in {"PATH","LANG","LC_ALL","LC_CTYPE","TMPDIR"}}
    def csv_text(rows, header=("id","customer","amount","status")):
        text=io.StringIO();writer=csv.writer(text);writer.writerow(header);writer.writerows(rows);return text.getvalue()
    valid = [
        ("duplicates-and-decimals", [[("a","c1","0.10","paid"),("b","c1","0.20","paid"),("a","c1","0.10","paid"),("c","c2","9.99","cancelled")]], {"customers":{"c1":30},"total_cents":30,"paid_orders":2}),
        ("large-exact-money", [[("a","c2","12345678901234567890.45","paid"),("b","c1","0.01","paid")]], {"customers":{"c1":1,"c2":1234567890123456789045},"total_cents":1234567890123456789046,"paid_orders":2}),
        ("decimal-widths-and-quoted-customer", [[("a","x,quoted","42","paid"),("b","x,quoted","1.2","paid")],[("a","x,quoted","42","paid"),("c","z","0.01","paid")]], {"customers":{"x,quoted":4320,"z":1},"total_cents":4321,"paid_orders":3}),
    ]
    invalid = [("negative",csv_text([("a","c","-0.01","paid")])),
        ("nan",csv_text([("a","c","NaN","paid")])),
        ("infinity",csv_text([("a","c","Infinity","paid")])),
        ("fractional-cent",csv_text([("a","c","0.001","paid")])),
        ("malformed-money",csv_text([("a","c","not-money","paid")])),
        ("unknown-status",csv_text([("a","c","1.00","__UNKNOWN_STATUS__")])),
        ("missing-column-without-rows",csv_text([],header=("id","customer","amount"))),
        ("cancelled-invalid-money",csv_text([("a","c","not-money","cancelled")])),
        ("conflicting-id",csv_text([("a","c","1.00","paid"),("a","c","2.00","paid")])),
    ]
    checks=[]
    for name,files,expected in valid:
        folder=audit/name;folder.mkdir();inp=folder/"input";inp.mkdir();out=folder/"output.json"
        for i,rows in enumerate(files):(inp/f"part-{i}.csv").write_text(csv_text(rows))
        observations=[]
        for _ in range(2):
            ran=subprocess.run([sys.executable,"-I",str(project/"reconcile.py"),"--input",str(inp),"--output",str(out)],capture_output=True,text=True,timeout=20,env=env)
            try: actual=json.loads(out.read_text())
            except (OSError,ValueError):actual=None
            observations.append(ran.returncode==0 and actual==expected and list(actual["customers"])==sorted(actual["customers"]))
        checks.append({"name":name,"passed":all(observations),"idempotent_runs":2})
    for name,body in invalid:
        folder=audit/name;folder.mkdir();inp=folder/"input";inp.mkdir();out=folder/"output.json"
        (inp/"bad.csv").write_text(body);original=b'{"preserve":true}\n';out.write_bytes(original)
        ran=subprocess.run([sys.executable,"-I",str(project/"reconcile.py"),"--input",str(inp),"--output",str(out)],capture_output=True,text=True,timeout=20,env=env)
        checks.append({"name":name,"passed":ran.returncode==2 and out.read_bytes()==original,"exit_code":ran.returncode,"original_output_preserved":out.read_bytes()==original})
    (audit/"receipt.json").write_text(json.dumps({"passed":all(c["passed"] for c in checks),"checks":checks},indent=2))
    return checks


def memory_case(binary, root, model, endpoint):
    project, env, config = fixture(root, model, endpoint)
    apply_config(binary, project, env, config, root)
    a,b="PRIVATE_A_"+uuid.uuid4().hex,"PRIVATE_B_"+uuid.uuid4().hex
    prompts=[("memory-a",f"请用 memory_save 保存一条仅属于本会话的记忆：{a}。scope 必须 conversation，kind reference。不要写文件、用shell或全局记忆。只报告工具实际结果。"),
        ("memory-b",f"这是独立会话。用 memory_search 在 conversation 范围查 PRIVATE_A_，不猜测内容；再用 memory_save 仅在本会话保存 {b}，kind reference。禁止文件/shell/全局记忆或代理。"),
        ("memory-a","恢复我们的会话。用 memory_search 仅在 conversation 范围查 PRIVATE_，报告实际返回的全部标记；禁止文件、shell、全局搜索或代理。"),
        ("memory-b","恢复我们的会话。用 memory_search 仅在 conversation 范围查 PRIVATE_，报告实际返回的全部标记；禁止文件、shell、全局搜索或代理。")]
    turns=[]
    for i,(session,prompt) in enumerate(prompts):
        result=command(binary,project,env,root/f"memory-{i}",prompt,session,iterations=12)
        require_completed(result);turns.append(result)
    with sqlite3.connect(Path(env["NOLA_DB_PATH"]).resolve().as_uri()+"?mode=ro",uri=True) as db:
        records=db.execute("SELECT content,owner_json FROM memory_records WHERE deleted_at_ms IS NULL").fetchall()
    assert {(content,json.loads(owner).get('conversation_id')) for content,owner in records}=={(a,"memory-a"),(b,"memory-b")}
    assert a in turns[2]["assistant"] and b not in turns[2]["assistant"]
    assert b in turns[3]["assistant"] and a not in turns[3]["assistant"]
    return {"passed":True,"turns":turns}


def cautious_case(binary, root, model, endpoint):
    project,env,config=fixture(root,model,endpoint,"cautious");apply_config(binary,project,env,config,root)
    target=project/"existing.txt";target.write_text("ORIGINAL\n")
    result=command(binary,project,env,root/"permission",
        "把 existing.txt 完整覆盖为 REPLACED 一行。先读取；如果需要权限批准，应停在权限等待并准确说明用户接下来怎样批准，不能改用shell或其他工具绕过。不要建替代文件，不要代理或联网。", "permission-a",iterations=10,timeout=180,allow_pending=True)
    assert target.read_text()=="ORIGINAL\n"
    assert not result["timed_out"] and result.get("durable_waits"),result
    cold = subprocess.run([str(binary), "--json", "session", "show", "permission-a"],
        cwd=project, env=env, capture_output=True, text=True, timeout=30)
    assert cold.returncode == 0
    projection = json.loads(cold.stdout)
    (root/"permission-projection.json").write_text(json.dumps(projection,ensure_ascii=False,indent=2))
    assert projection["permission_projection"]["pending"], "Permission wait disappeared on cold read"
    return {"passed":True,"permission_pending":True,"turns":[result]}


def background_case(binary, root, model, endpoint):
    project,env,config=fixture(root,model,endpoint);apply_config(binary,project,env,config,root)
    nonce="JOB_PRIVATE_"+uuid.uuid4().hex
    program = """import json,time
from pathlib import Path
with Path('starts.txt').open('a') as f: f.write('start\\n')
print(__NONCE__,flush=True)
for i in range(1,7):
    time.sleep(10)
    with Path('progress.txt').open('a') as f: f.write(str(i)+'\\n')
Path('result.json.tmp').write_text(json.dumps({'nonce':__NONCE__,'steps':6,'total':21}))
Path('result.json.tmp').replace('result.json')
""".replace("__NONCE__", repr(nonce))
    (project/"job.py").write_text(program)
    first=command(binary,project,env,root/"background-start",
        "只启动一次后台任务：用 bash 执行 python3 job.py，execution_mode=background，timeout_sec=120。"
        "读一次真实返回 task_id 的 read_task 后就结束本轮，只报告正在运行或真实状态，不要等它完成、不重启、不要代理、不要改文件或联网。",
        "background-a",timeout=180,iterations=12)
    require_completed(first)
    with sqlite3.connect((root/"nola.db").resolve().as_uri()+"?mode=ro",uri=True) as db:
        tasks=db.execute("SELECT id,owner_session_id,status FROM durable_tasks WHERE kind='shell_process'").fetchall()
    assert len(tasks)==1 and tasks[0][1]=="background-a",tasks
    task_id=tasks[0][0]
    assert (project/"starts.txt").read_text()=="start\n"
    foreign=command(binary,project,env,root/"background-foreign",
        f"这是另一个会话的任务所有权验收。请只对 {task_id} 调用 read_task 和 stop_task 各一次，并报告真实拒绝。不要调用文件或shell工具，不要联网、代理、改文件或用其他路由绕过。",
        "background-b",timeout=180,iterations=12)
    assert foreign["exit_code"]==0 and not foreign["timed_out"] and not foreign.get("audit_error")
    assert set(foreign["canonical"]["outcomes"]) <= {"answered","blocked"}
    assert nonce not in (root/"background-foreign/summary.json").read_text(), "Foreign task output leaked"
    events=[json.loads(line) for line in (root/"background-foreign/events.ndjson").read_text().splitlines()]
    for name in ["read_task","stop_task"]:
        calls=[event["trace"] for event in events if event.get("trace",{}).get("kind")=="tool.completed" and event["trace"].get("payload",{}).get("tool_name")==name]
        assert len(calls)==1 and calls[0].get("status")=="failed", "Foreign task operation was not refused"
    final=command(binary,project,env,root/"background-owner-resume",
        "继续我们启动过的那个后台任务。用任务读取/等待工具观察同一个任务直到完成，再读取 result.json 核验实际结果。不要重新执行bash、不要重启任务、不派生代理、不改文件、不联网。完成与否按工具实际返回报告。",
        "background-a",timeout=240,iterations=16)
    require_completed(final)
    assert json.loads((project/"result.json").read_text())=={"nonce":nonce,"steps":6,"total":21}
    assert (project/"starts.txt").read_text()=="start\n", "Background command was executed more than once"
    assert (project/"progress.txt").read_text().splitlines()==[str(i) for i in range(1,7)]
    with sqlite3.connect((root/"nola.db").resolve().as_uri()+"?mode=ro",uri=True) as db:
        terminal=db.execute("SELECT id,owner_session_id,status FROM durable_tasks WHERE kind='shell_process'").fetchall()
    assert terminal==[(task_id,"background-a","completed")],terminal
    return {"passed":True,"task_id":task_id,"single_execution":True,"foreign_read_stop_refused":True,"turns":[first,foreign,final]}


def compaction_case(binary, root, model, endpoint):
    result = chain_case(binary, root, model, endpoint, threshold=18000, seed_history=True)
    with sqlite3.connect((root/"nola.db").resolve().as_uri()+"?mode=ro",uri=True) as db:
        events = [json.loads(row[0]) for row in db.execute(
            "SELECT payload_json FROM run_events WHERE event_kind='compaction_completed'")]
    actual = [event for event in events if event.get("replaced_item_count",0)>0 and event.get("compaction_item_id")]
    assert actual, "No real nonempty compaction completed"
    result["completed_compactions"] = len(actual)
    return result


def apply_config(binary, project, env, config, root):
    result=subprocess.run([str(binary),"model","apply-config",str(config)],cwd=project,env=env,capture_output=True,text=True,timeout=30)
    (root/"apply.stdout").write_text(result.stdout);(root/"apply.stderr").write_text(result.stderr)
    assert result.returncode==0,"Current-format model configuration failed to apply"


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--binary',type=Path,default=Path('target/debug/nola'))
    parser.add_argument('--config',type=Path,default=Path.home()/'.nola/config.json')
    parser.add_argument('--case',choices=['chain','pipeline','memory','cautious','compaction','background','all'],default='all')
    args=parser.parse_args();binary=args.binary.resolve();upstream,model,secret=selected_route(args.config)
    root=Path(tempfile.mkdtemp(prefix='nola-real-long-'));root.chmod(0o700)
    print('Evidence:',root,flush=True)
    report={'model':model,'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'cases':{},'passed':False}
    try:
        with observed_provider(upstream,model,secret,root/'provider.ndjson') as endpoint:
            for name,function in [('pipeline',pipeline_case),('chain',chain_case),('memory',memory_case),('cautious',cautious_case),('compaction',compaction_case),('background',background_case)]:
                if args.case in {'all',name}:
                    try:report['cases'][name]=function(binary,root/name,model,endpoint)
                    except Exception as error:
                        report['cases'][name]={'passed':False,'error':str(error)}
                        print('FAILED:',name,type(error).__name__,flush=True)
                    (root/'receipt.json').write_text(json.dumps(report,ensure_ascii=False,indent=2))
        records=[json.loads(line) for line in (root/'provider.ndjson').read_text().splitlines()]
        real=[r for r in records if r.get('forwarded') is not False]
        report['provider_requests']=len(real)
        report['relay_metadata_rejections']=len(records)-len(real)
        report['successful_provider_requests']=sum(200<=r.get('status',0)<300 for r in real)
        if args.case == 'cautious':
            # A real permission wait can stop at a tool call without assistant text.
            report['observed_text_streams'] = 0
        else:
            report['observed_text_streams'] = require_text_stream(real)
        report['passed'] = (bool(report['cases']) and all(case['passed'] for case in report['cases'].values())
            and report['successful_provider_requests'] == len(real)
            and all(not r.get('injectedInterruption') and not r.get('injectedHttpError')
                and not r.get('injectedCompactionTruncation') for r in real))
        (root/'receipt.json').write_text(json.dumps(report,ensure_ascii=False,indent=2))
        # No credential may persist in retained evidence, including nested event logs.
        secrets = {secret} | {value for key, value in os.environ.items()
            if value and (key.endswith("_API_KEY") or key == "FREE_MODEL_KEY")}
        for path in root.rglob("*"):
            if path.is_file() and any(value.encode() in path.read_bytes() for value in secrets):
                raise RuntimeError('Credential appeared in evidence; do not publish artifacts')
        print(json.dumps({'passed':report['passed'],'provider_requests':report['provider_requests'],'cases':{k:v['passed'] for k,v in report['cases'].items()},'evidence':str(root)},ensure_ascii=False),flush=True)
        return 0 if report['passed'] else 1
    finally:
        secret=None

if __name__=='__main__':raise SystemExit(main())
