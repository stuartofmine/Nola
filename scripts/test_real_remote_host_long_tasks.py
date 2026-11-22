#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Actual browser/Host/provider long-run acceptance, with independent Store/file oracles.

No production database/configuration mutation, response fabrication or new keys.
Use --smoke to validate this harness; it never counts as hundreds-step evidence.
"""
import argparse
import json
import os
from pathlib import Path
import signal
import socket
import sqlite3
import subprocess
import tempfile
import time
import uuid
from urllib.request import urlopen
from urllib.parse import urlsplit

from canonical_run_audit import audit_runs
from real_e2e_events import committed_journal_parts
from test_real_cli_hundreds_steps import make_chain, chain_prompt, check_files, provider_totals
from test_real_cli_long_tasks import selected_route, fixture, apply_config, observed_provider

ROOT = Path(__file__).resolve().parents[1]


def committed_tools(database, conversation, run_id=None):
    with sqlite3.connect(database.as_uri() + '?mode=ro', uri=True) as db:
        headers = [json.loads(raw) for (raw,) in db.execute('''SELECT r.header_json FROM assistant_journal_records r
            WHERE r.sequence=(SELECT max(sequence) FROM assistant_journal_records WHERE owner=r.owner)
            AND json_extract(r.owner,'$.conversation_id')=?
            AND (? IS NULL OR json_extract(r.owner,'$.run_id')=?)''', (conversation, run_id, run_id))]
    events = [{'type': 'assistant_journal_committed_v1', 'through': {
        name: header[name] for name in ('identity', 'sequence', 'record_digest')}} for header in headers]
    result = []
    for part in committed_journal_parts(events, database):
        metadata = part['head']['metadata']
        if metadata['kind'] != 'tool':
            continue
        tool = metadata['data']
        encoded = json.loads(part['bodies']['tool_input'])
        if encoded.get('protocol_version') != 1 or not isinstance(encoded.get('value'), dict):
            raise ValueError('Current committed tool input required')
        entry = dict(tool=tool['capability_id'], state=tool['state'], call_id=tool['tool_call_id'],
                     arguments=encoded['value'])
        if entry['tool'] == 'read_file' and Path(entry['arguments'].get('path','')).name == 'report.json':
            raw = part['bodies'].get('tool_output')
            if raw:
                entry['output'] = json.loads(raw)['value']
        result.append(entry)
    return result


def verify_chain_reads(project, expected, tools):
    """Count committed node reads and require a real read-back of the final write."""
    node_reads = []
    report_verified = False
    report_written = False
    report_content = bytearray()
    next_line = 1
    missing_confirmed = False
    recovered = False
    for tool in tools:
        if tool['tool'] not in {'read_file', 'write_file'}:
            continue
        target = Path(tool['arguments'].get('path', ''))
        name = (project / target).resolve().relative_to(project.resolve()).as_posix()
        if tool['tool'] == 'read_file' and name == expected['missing'] and tool['state'] == 'failed':
            missing_confirmed = True
        if tool['tool'] == 'read_file' and name == expected['recovery'] and tool['state'] == 'completed':
            assert missing_confirmed, 'Read the recovery file before confirming the missing link'
            recovered = True
        if name == 'report.json':
            if tool['tool'] == 'write_file':
                report_written = tool['state'] == 'completed'
                report_verified = False
                report_content.clear()
                next_line = 1
            elif report_written and tool['state'] == 'completed':
                output = tool.get('output', {})
                content = output.get('content')
                if isinstance(content, str):
                    offset = output.get('start_byte')
                    line = tool['arguments'].get('start_line', 1) if offset is None else None
                    if output.get('complete') or offset == 0 or line == 1:
                        report_content.clear()
                        next_line = 1
                    if output.get('complete') or offset == len(report_content) or (line is not None and line == next_line):
                        report_content.extend(content.encode())
                        next_line = output.get('next_start_line')
                        if output.get('complete') or output.get('eof') or (line is not None and next_line is None):
                            report_verified |= report_content == (project / name).read_bytes()
                if any(check.get('passed') and check.get('predicate') == 'json_value_matches_expected'
                       for check in output.get('_nola_execution', {}).get('checks', [])):
                    report_verified |= json.loads(tool['arguments']['expected_json']) == json.loads((project / name).read_bytes())
        if tool['tool'] == 'read_file' and name in expected['names']:
            assert tool['state'] == 'completed', 'Node read failed'
            node_reads.append(name)
    assert list(dict.fromkeys(node_reads)) == expected['names'], 'Committed node read order/coverage mismatch'
    assert recovered, 'Missing confirmed recovery from the broken link'
    assert report_verified, 'Missing complete report content or successful exact JSON validation after the final write'
    return node_reads


def checkpoint_schedule(project, expected, tools):
    nodes = set(expected['names'])
    seen = set()
    writes = []
    for tool in tools:
        if tool['state'] != 'completed' or tool['tool'] not in {'read_file', 'write_file'}:
            continue
        target = (project / tool['arguments']['path']).resolve().relative_to(project.resolve())
        if tool['tool'] == 'read_file' and target.as_posix() in nodes:
            seen.add(target.as_posix())
        elif tool['tool'] == 'write_file' and target.parent == Path('checkpoints'):
            writes.append((target.name, len(seen)))
    required = [(f'{start // 20:03}.json', min(start + 20, len(nodes))) for start in range(0, len(nodes), 20)]
    return dict(actual=writes, required=required, on_time=writes == required)


def cold_followup(root, config_path, binary):
    """Restart the owned Host/relay without changing profiles or the original receipt."""
    root = root.resolve()
    assert root.name.startswith('nola-remote-long-') and (root/'receipt.json').is_file()
    original = json.loads((root/'receipt.json').read_text())
    assert original['passed'], 'Original acceptance must pass first'
    upstream, model, secret = selected_route(config_path)
    connection = json.loads((root/'store/home/.nola/config.json').read_text())['llm']['connections'][0]
    assert connection['models'][0]['id'] == model
    relay_url = urlsplit(connection['endpoint'])
    assert relay_url.hostname == '127.0.0.1' and relay_url.port
    from e2e.real_overwrite_permission import isolated_environment
    env = isolated_environment(root/'store', 'cautious')
    allowed = {'PATH','HOME','USER','LOGNAME','SHELL','TMPDIR','LANG','LC_ALL','LC_CTYPE','TERM','TZ'}
    env = {key:value for key,value in env.items() if key in allowed or key.startswith('NOLA_')}
    env.update(NOLA_AUTOMATIC_MEMORY_MAINTENANCE_ENABLED='0', NOLA_RESIDENT_MEMORY_RECALL_ENABLED='0')
    project = root/'store/project'
    host = None
    result = {'passed':False, 'original_receipt_preserved':True}
    try:
        with observed_provider(upstream, model, secret, root/'provider-cold.ndjson', listen_port=relay_url.port):
            with socket.socket() as sock:
                sock.bind(('127.0.0.1',0)); port=sock.getsockname()[1]
            with (root/'host-cold.log').open('w') as log:
                host = subprocess.Popen([str(binary),'web','--host','127.0.0.1','--port',str(port),
                    '--web-root',str(ROOT/'apps/web'),'--project',str(project),'--allow-dir',str(project),'--no-skills'],
                    cwd=project,env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
                url=f'http://127.0.0.1:{port}'
                deadline=time.monotonic()+30
                while time.monotonic()<deadline:
                    try: urlopen(url+'/login',timeout=2).close(); break
                    except OSError:
                        assert host.poll() is None, 'Cold Host exited'; time.sleep(.1)
                else: raise TimeoutError('Cold Host startup')
                browser_env={k:v for k,v in os.environ.items() if k in {'PATH','HOME','TMPDIR','LANG','USER','SHELL'}}
                browser_env.update(NOLA_REMOTE_LONG_OUT=str(root),NOLA_REMOTE_LONG_URL=url)
                with (root/'browser-cold.log').open('w') as output:
                    process=subprocess.run(['node',str(ROOT/'apps/web/playwright_runtime/remote_host_cold_followup.cjs')],
                        env=browser_env,cwd=ROOT,stdout=output,stderr=subprocess.STDOUT,timeout=260)
                ui=json.loads((root/'cold-browser-receipt.json').read_text())
                assert process.returncode==0 and ui['passed'], 'Cold context recall failed; inspect private receipt'
                with sqlite3.connect((root/'store/nola.db').as_uri()+'?mode=ro',uri=True) as db:
                    result['canonical']=audit_runs(db,original['browser']['chainConversation'])
                    result['nonempty_compactions']=sum(json.loads(raw).get('replaced_item_count',0)>0
                        for (raw,) in db.execute("SELECT payload_json FROM run_events WHERE run_id=? AND event_kind='compaction_completed'",
                                                (original['browser']['chainRun'],)))
                assert result['canonical']['runCount']==2 and all(x in {'answered','action_completed'} for x in result['canonical']['outcomes'])
                followup_tools = committed_tools(root/'store/nola.db',original['browser']['chainConversation'],ui['followupRun'])
                result['followup_tool_calls'] = len(followup_tools)
                assert not followup_tools, 'Cold follow-up used tools despite the explicit context-only request'
                if not original['smoke']:
                    assert result['nonempty_compactions'] > 0, 'Cold long-run scenario never compacted'
                expected=json.loads((root/'independent-oracle.json').read_text())
                result['files']=check_files(project,expected)
                result.update(passed=True,browser=ui,provider=provider_totals(root/'provider-cold.ndjson'))
    except Exception as error: result['error']=str(error)
    finally:
        if host is not None and host.poll() is None:
            os.killpg(host.pid,signal.SIGTERM)
            try:host.wait(timeout=8)
            except subprocess.TimeoutExpired:os.killpg(host.pid,signal.SIGKILL);host.wait(timeout=5)
        (root/'cold-receipt.json').write_text(json.dumps(result,indent=2))
    print(json.dumps({'passed':result['passed'],'evidence':str(root),'error':result.get('error')}),flush=True)
    return 0 if result['passed'] else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--resume-evidence',type=Path,help='Cold-restart an already-passed owned acceptance Store and check context recall')
    parser.add_argument('--config', type=Path, default=Path.home()/'.nola/config.json')
    parser.add_argument('--binary', type=Path, default=ROOT/'target/debug/nola')
    parser.add_argument('--nodes', type=int, default=200)
    parser.add_argument('--smoke', action='store_true')
    parser.add_argument('--timeout', type=int, default=3600)
    args = parser.parse_args()
    if args.resume_evidence:
        return cold_followup(args.resume_evidence,args.config,args.binary)
    count = 4 if args.smoke else args.nodes
    if not args.smoke and not 200 <= count <= 500:
        parser.error('Hundreds-step acceptance requires 200–500 nodes; use --smoke for harness checks')
    if not 120 <= args.timeout <= 7200:
        parser.error('Timeout must be 120–7200 seconds')
    upstream, model, secret = selected_route(args.config)
    root = Path(tempfile.mkdtemp(prefix='nola-remote-long-')); root.chmod(0o700)
    print(json.dumps({'stage':'started', 'evidence':str(root), 'nodes':count, 'smoke':args.smoke}), flush=True)
    receipt = dict(passed=False, smoke=args.smoke, nodes=count)
    browser = host = None
    try:
        with observed_provider(upstream, model, secret, root/'provider.ndjson') as endpoint:
            project, env, config = fixture(root/'store', model, endpoint, profile='cautious')
            apply_config(args.binary, project, env, config, root/'store')
            expected = make_chain(project, count)
            marker = 'REMOTE_FACT_'+uuid.uuid4().hex
            (root/'independent-oracle.json').write_text(json.dumps(expected))
            prompt = chain_prompt(expected, marker) + ' 所有输出文件仅使用 write_file 创建。最终回复以 REMOTE_CHAIN_DONE 开头。'
            if args.smoke:
                prompt += ' 开始前请实际调用 task_scope_update，以当前用户请求为来源、report.json 为必需资源，登记上述任务范围。创建每份 JSON 时在 write_file 中声明 content_format=json，写入前检验整个内容。'
            (root/'browser-input.json').write_text(json.dumps(dict(prompt=prompt,nodes=count,timeoutMs=args.timeout*1000)))
            (project/'cancel_job.py').write_text("from pathlib import Path\nimport time\nwith Path('cancel-starts.txt').open('a') as f:f.write('start\\n')\nfor i in range(180):\n with Path('cancel-progress.txt').open('a') as f:f.write(str(i)+'\\n')\n time.sleep(1)\nPath('cancel-result.json').write_text('completed')\n")
            with socket.socket() as sock:
                sock.bind(('127.0.0.1',0)); port=sock.getsockname()[1]
            host_log=(root/'host.log').open('w')
            host=subprocess.Popen([str(args.binary),'web','--host','127.0.0.1','--port',str(port),
                '--web-root',str(ROOT/'apps/web'),'--project',str(project),'--allow-dir',str(project),'--no-skills'],
                cwd=project,env=env,stdout=host_log,stderr=subprocess.STDOUT,start_new_session=True)
            url=f'http://127.0.0.1:{port}'
            deadline=time.monotonic()+30
            while time.monotonic()<deadline:
                try:
                    urlopen(url+'/login',timeout=2).close(); break
                except OSError:
                    if host.poll() is not None:raise RuntimeError('Isolated Host exited')
                    time.sleep(.1)
            else:raise TimeoutError('Host startup timed out')
            browser_env={k:v for k,v in os.environ.items() if k in {'PATH','HOME','TMPDIR','LANG','USER','SHELL','TERM'}}
            browser_env.update(NOLA_REMOTE_LONG_OUT=str(root),NOLA_REMOTE_LONG_URL=url)
            output=(root/'browser.log').open('w')
            browser=subprocess.Popen(['node',str(ROOT/'apps/web/playwright_runtime/remote_host_hundreds_steps.cjs')],
                env=browser_env,cwd=ROOT,stdout=output,stderr=subprocess.STDOUT,start_new_session=True)
            deadline=time.monotonic()+args.timeout+420
            last=0
            while browser.poll() is None:
                now=time.monotonic()
                if now-last>=15:
                    totals=provider_totals(root/'provider.ndjson')
                    print(json.dumps({'stage':'running','provider_requests':totals['requests']}),flush=True)
                    if totals['requests']>count*2+80 or totals['input_tokens']>14_000_000 or totals['output_tokens']>200_000:
                        raise RuntimeError('Acceptance request/token limit reached')
                    last=now
                if now>deadline:raise TimeoutError('Acceptance deadline')
                time.sleep(1)
            output.close()
            ui=json.loads((root/'browser-receipt.json').read_text())
            receipt['browser']=ui
            if browser.returncode or not ui['passed']:raise RuntimeError('Browser acceptance failed; see private receipt')
            receipt['files']=check_files(project,expected)
            tools=committed_tools(root/'store/nola.db',ui['chainConversation'])
            reads = verify_chain_reads(project, expected, tools)
            assert not any(t['tool'] in {'bash','code_mode_exec','Task'} for t in tools), 'Audit bypassed sequential reads'
            with sqlite3.connect((root/'store/nola.db').as_uri()+'?mode=ro',uri=True) as db:
                admitted = db.execute('''SELECT r.user_message_id, e.payload_json FROM runs r
                    JOIN run_events e ON e.run_id=r.id AND e.event_kind='input_accepted'
                    WHERE r.id=? ORDER BY e.sequence LIMIT 1''', (ui['chainRun'],)).fetchone()
                assert admitted and admitted[0] in json.loads(admitted[1])['admission']['item_ids'], 'Applied input lost its canonical message identity'
                if args.smoke:
                    writes = [t for t in tools if t['tool'] == 'write_file' and t['state'] == 'completed']
                    assert writes and all(t['arguments'].get('content_format') == 'json' for t in writes), 'JSON files were created without their declared format check'
                    revisions = db.execute("SELECT count(*) FROM run_events WHERE run_id=? AND event_kind='requirement_scope_changed'",
                        (ui['chainRun'],)).fetchone()[0]
                    assert revisions >= 2, 'Model scope revision did not commit'
                receipt['chain']=audit_runs(db,ui['chainConversation'])
                receipt['cancel']=audit_runs(db,ui['cancelConversation'])
            assert receipt['chain']['runCount']==1 and receipt['chain']['outcomes'][0] in {'answered','action_completed'}
            assert receipt['cancel']['outcomes']==['cancelled','answered']
            receipt['committed_node_reads']=len(reads)
            receipt['checkpoint_schedule'] = checkpoint_schedule(project, expected, tools)
            receipt['provider']=provider_totals(root/'provider.ndjson')
            assert receipt['provider']['successful_requests']>=count
            assert receipt['provider']['injected_responses']==0
            assert receipt['checkpoint_schedule']['on_time'], 'Checkpoints were not each written once at their requested boundary'
            receipt['passed']=True
    except Exception as error:
        receipt['error']=str(error)
    finally:
        for process in (browser,host):
            if process is not None and process.poll() is None:
                os.killpg(process.pid,signal.SIGTERM)
                try:process.wait(timeout=8)
                except subprocess.TimeoutExpired:os.killpg(process.pid,signal.SIGKILL);process.wait(timeout=5)
        if 'host_log' in locals():host_log.close()
        (root/'receipt.json').write_text(json.dumps(receipt,indent=2))
    print(json.dumps({'passed':receipt['passed'],'evidence':str(root),'error':receipt.get('error')}),flush=True)
    return 0 if receipt['passed'] else 1


if __name__=='__main__':
    raise SystemExit(main())
