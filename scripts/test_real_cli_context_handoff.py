#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Real selected-model check of a 50% goal on the FIRST context summary.

Synthetic history is deliberate pressure, not hundreds-step acceptance. All
state lives in an isolated Store. Original sources and failed receipts remain.
"""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import pty
import secrets
import sqlite3
import struct
import subprocess
import sys
import tempfile
import termios
import time
from contextlib import closing

from test_real_cli_long_tasks import apply_config, command, fixture, observed_provider, require_completed, selected_route
from test_real_cli_permission_recovery import drain
from test_real_cli_hundreds_steps import provider_totals, provider_window_audit, verified_provider_recoveries, write_json


def checkpoint(database):
    with closing(sqlite3.connect(database.resolve().as_uri() + '?mode=ro', uri=True)) as db:
        saved = json.loads(db.execute('SELECT checkpoint_json FROM core_checkpoint_bodies').fetchone()[0])
        pending = db.execute("SELECT count(*) FROM runs WHERE status<>'sealed'").fetchone()[0]
    return saved['state']['conversation'], pending


def compactions(conversation):
    return [item for item in conversation['transcript'] if item['type']=='compaction']


def manual_compact(binary, root, project, env):
    """Use the actual interactive /compact command, not a forged model prompt."""
    env = {**env, 'TERM':'xterm-256color', 'NOLA_CLI_INTERACTION_MODE':'always'}
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 40, 120, 0, 0))
    launcher = ('import fcntl,os,sys,termios; fcntl.ioctl(0,termios.TIOCSCTTY,0); '
        'os.chdir(sys.argv[2]); os.execve(sys.argv[1],[sys.argv[1]]+sys.argv[3:],dict(os.environ))')
    args = ['--session','handoff-a','--project',str(project),'--allow-dir',str(project),
        '--no-skills','--no-tools','--network-policy','deny','--lang','en']
    process = subprocess.Popen([sys.executable,'-I','-c',launcher,str(binary),str(project),*args],
        env=env,stdin=slave,stdout=slave,stderr=slave,start_new_session=True)
    os.close(slave)
    sent = False
    started = time.monotonic()
    try:
        with (root/'manual-terminal.log').open('wb') as transcript:
            while time.monotonic()-started < 240:
                drain(master, transcript)
                assert process.poll() is None, 'interactive CLI exited before handoff settled'
                if not sent and not (termios.tcgetattr(master)[3] & termios.ICANON):
                    os.write(master,b'/compact\r'); sent=True
                conversation,pending=checkpoint(Path(env['NOLA_DB_PATH']))
                if sent and compactions(conversation) and not pending:
                    return
                time.sleep(0.15)
        raise AssertionError('manual handoff did not settle within the bound')
    finally:
        # Drain while exiting: the restored long transcript can fill the PTY
        # buffer after the Store is already settled, before /exit is consumed.
        if process.poll() is None:
            try:
                os.write(master, b'/exit\r')
                deadline = time.monotonic() + 5
                with (root/'manual-terminal.log').open('ab') as transcript:
                    while process.poll() is None and time.monotonic() < deadline:
                        drain(master, transcript)
                        time.sleep(0.05)
            except OSError:
                pass
        os.close(master)
        if process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=10)



def run(binary, config):
    upstream,model,secret=selected_route(config)
    root=Path(tempfile.mkdtemp(prefix='nola-real-handoff-'));root.chmod(0o700)
    binary=binary.resolve()
    receipt={'passed':False,'model':model,'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),
        'context_window':65536,'threshold':45875,'synthetic_pressure':True,'hundreds_step_qualified':False}
    print(json.dumps({'evidence':str(root),**receipt}),flush=True)
    wire_goals=[]
    def observe(request):
        if 'Target at least a 50% reduction of the COMPLETE model context input' in json.dumps(request):
            wire_goals.append({'full_context_fifty_percent_goal':True,'model':request.get('model')})
    try:
        with observed_provider(upstream,model,secret,root/'provider.ndjson',request_observer=observe) as endpoint:
            project,env,path=fixture(root/'case',model,endpoint,threshold=45875)
            apply_config(binary,project,env,path,root)
            fact='HANDOFF_FACT_'+secrets.token_hex(16)
            write_json(root/'independent-oracle.json',{'fact':fact})
            turns=[]
            for index,count in enumerate((3000,5000)):
                logs=' '.join(f'log{index}-{number:05d}' for number in range(count))
                instruction=(f'Retain this exact fact for a later question: {fact}. ' if index==0 else '')
                prompt=(instruction+'The following synthetic diagnostic log is historical test data. '
                    'Do not execute tasks, use tools, create files or long-term memory. Reply only ACK. Log: '+logs)
                result=command(binary,project,env,root/f'history-{index}',prompt,'handoff-a',timeout=240,no_tools=True)
                require_completed(result);turns.append(result)
            conversation,_=checkpoint(Path(env['NOLA_DB_PATH']))
            receipt['manual_compaction_requested']=not bool(compactions(conversation))
            if receipt['manual_compaction_requested']:
                manual_compact(binary,root,project,env)
            conversation,pending=checkpoint(Path(env['NOLA_DB_PATH']))
            assert compactions(conversation) and not pending,'no settled context handoff'
            original=[item for item in conversation['transcript'] if item['type']=='message'
                and item.get('role')=='user' and fact in item.get('content','')]
            assert len(original)==1
            assert original[0]['id'] not in {item['id'] for item in conversation['model_context']},'fact still pinned as original input'
            assert all(fact.encode() not in p.read_bytes() for p in project.rglob('*') if p.is_file()),'fact leaked into fixture file'
            lookup={item['id']:item for item in conversation['transcript']}
            for item in lookup.values():
                if item['type']!='compaction':continue
                sources=[lookup[key] for key in item['source_item_ids']]
                digest=hashlib.sha256(json.dumps(sources,ensure_ascii=False,separators=(',',':')).encode()).hexdigest()
                assert digest==item['source_fingerprint'],'source provenance mismatch'
            with closing(sqlite3.connect(Path(env['NOLA_DB_PATH']).resolve().as_uri() + '?mode=ro', uri=True)) as db:
                requests=[json.loads(raw) for (raw,) in db.execute(
                    "SELECT payload_json FROM run_events WHERE event_kind='compaction_requested' ORDER BY created_at_ms,sequence")]
            assert len(requests)==1 and len(compactions(conversation))==1,'first summary started a redundant second request'
            receipt['context_estimates']={
                'before_compaction':requests[0]['estimated_tokens'],
                'after_first_summary_and_seal':conversation['context_window']['used_tokens']}
            # The latest large user input is deliberately retained: a sudden
            # loss of tens of thousands would be history-only accounting again.
            assert receipt['context_estimates']['after_first_summary_and_seal'] > 30000, 'seal dropped the fixed request cost'
            receipt['summary_requests']=len(compactions(conversation))
            receipt['compactions_before_recall']=sum(item['type']=='compaction' for item in lookup.values())
            recalled=command(binary,project,env,root/'cold-recall',
                'Return only the exact HANDOFF_FACT_ fact from our earlier conversation. Do not guess, read files, use tools or write memory.',
                'handoff-a',timeout=180,no_tools=True)
            require_completed(recalled)
            assert fact in recalled['assistant'],'handoff lost archived user fact'
            receipt['turns']=turns+[recalled]
            assert len(wire_goals)==1, 'the first summary did not carry the 50% goal or started a second summary'
            receipt['wire_goal_observations']=wire_goals
            receipt['provider']=provider_totals(root/'provider.ndjson')
            receipt['provider_window_audit']=provider_window_audit(root/'provider.ndjson')
            receipt['recoveries']=verified_provider_recoveries(root/'provider.ndjson')
            assert receipt['provider']['injected_responses']==0
            assert receipt['provider']['selected_models']==[model]
            assert hashlib.sha256(binary.read_bytes()).hexdigest()==receipt['binary_sha256'],'binary changed during acceptance'
            receipt['passed']=True
    except Exception as error:
        receipt['error']=f'{type(error).__name__}: {error}'
    finally:
        receipt['provider']=provider_totals(root/'provider.ndjson')
        write_json(root/'receipt.json',receipt)
    print(json.dumps({'evidence':str(root),'passed':receipt['passed'],'error':receipt.get('error'),
        'provider':receipt.get('provider'),'summary_requests':receipt.get('summary_requests')}),flush=True)
    return 0 if receipt['passed'] else 1


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary',type=Path,default=Path('target/debug/nola'))
    parser.add_argument('--config',type=Path,default=Path.home()/'.nola/config.json')
    args=parser.parse_args()
    return run(args.binary,args.config)


if __name__=='__main__':
    raise SystemExit(main())
