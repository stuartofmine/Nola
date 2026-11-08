#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Actual terminal acceptance, isolated Store; explicit deterministic HTTP fixture."""
from contextlib import closing
import hashlib
import http.server
import json
import os
from pathlib import Path
import re
import signal
import sqlite3
import sys
import tempfile
import threading
import time
import subprocess
import unittest
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from test_real_cli_terminal import Oracle, RealTerminal
from test_real_agent_room_isolation import start_service, stop_service, command
from e2e.real_overwrite_permission import isolated_environment, scrub_case_credentials

@unittest.skipUnless(os.environ.get('NOLA_HARNESS_TEST_BINARY'), 'explicit binary required')
class RemainingAcceptance(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp(prefix='nola-remaining-')).resolve()
        self.root.chmod(0o700)
        print(f'{self._testMethodName}: {self.root}', flush=True)
        self.binary = Path(os.environ['NOLA_HARNESS_TEST_BINARY']).resolve()
        self.project = self.root / 'workspace'; self.project.mkdir()
        self.env = isolated_environment(self.root, 'allow_all')
        self.env.update(TERM='xterm-256color', NOLA_CLI_INTERACTION_MODE='always', NOLA_LONG_TERM_MEMORY_ENABLED='0', NOLA_LLM_MAX_RETRIES='0')
        self.service_binary=self.binary.parent/'nola-service'
        self.report = {'passed': False, 'binary_sha256': hashlib.sha256(self.binary.read_bytes()).hexdigest(),
                       'service_sha256': hashlib.sha256(self.service_binary.read_bytes()).hexdigest(),
                       'model_mode':'controlled_http_fixture'}
        self.term = self.oracle = self.service = None
        self.requests = []; self.release = threading.Event(); self.truncate_stream = False
        outer = self
        class Provider(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_): pass
            def do_POST(self):
                body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                outer.requests.append(body)
                self.send_response(200); self.send_header('Content-Type', 'text/event-stream'); self.end_headers()
                try:
                    if outer.truncate_stream:
                        self.wfile.write(b'data: {"choices":[{"index":0,"delta":{"content":"UNCONFIRMED_PARTIAL"},"finish_reason":null}]}\n\n')
                        self.wfile.flush()
                        self.close_connection=True
                        return
                    text = json.dumps(body.get('messages', []), ensure_ascii=False)
                    if 'HOLD_A' in text and len(outer.requests) == 1:
                        outer.release.wait(90)
                    answer = 'ANSWER_B' if 'INPUT_B' in text else 'ANSWER_A'
                    for delta, finish in [({'role':'assistant','content':answer},None), ({},'stop')]:
                        self.wfile.write(('data: '+json.dumps({'choices':[{'index':0,'delta':delta,'finish_reason':finish}]})+'\n\n').encode()); self.wfile.flush()
                    self.wfile.write(b'data: [DONE]\n\n'); self.wfile.flush()
                except (BrokenPipeError, ConnectionResetError): pass
        self.server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Provider)
        self.server.daemon_threads = False
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True); self.thread.start()
        self.addCleanup(self.cleanup)
        config = self.root / 'home/.nola/config.json'; config.parent.mkdir(parents=True, exist_ok=True)
        config.write_text(json.dumps({'llm': {'connections': [{'id':'fixture','name':'Fixture',
            'endpoint':f'http://127.0.0.1:{self.server.server_port}/v1', 'protocol':'chat_completions',
            'models':[{'id':'fixture','name':'Fixture'}]}], 'default_model':{'connection':'fixture','model':'fixture'}}}))
        self.service, self.sock = start_service(self.service_binary, self.root, self.env)
        subprocess.run([str(self.binary),'model','apply-config',str(config)],env=self.env,stdout=subprocess.DEVNULL,check=True,timeout=30)
        self.selection = 'configured:'+self.query('SELECT id FROM model_profiles WHERE selected=1')[0][0]

    def query(self, sql, args=()):
        with closing(sqlite3.connect((self.root/'nola.db').as_uri()+'?mode=ro', uri=True)) as db:
            return db.execute(sql,args).fetchall()
    def view(self, session):
        scope=hashlib.sha256(str((self.root/'nola.db').resolve()).encode()).hexdigest()
        path=self.root/'CliViewState'/scope/(hashlib.sha256(session.encode()).hexdigest()+'.json')
        return json.loads(path.read_text())['state'] if path.exists() else {}
    def seed(self, name, count):
        command(self.sock, {'type':'create_conversation','input':{'id':name,'title':name,'settings_json':json.dumps({'project_root':str(self.project)}),'created_at_ms':1000,'updated_at_ms':1000}})
        command(self.sock, {'type':'patch_conversation_configuration','input':{'conversation_id':name,'expected_revision':1,'patch_json':json.dumps({'modelSelectionID':self.selection,'modelOverrides':{self.selection:{}},'permissionProfile':'allow_all'}),'updated_at_ms':1500}})
        if count:
            messages = []
            for i in range(count):
                messages.append({'id':f'{name}-message-{i}','run_id':None,'role':'user','status':'completed','ordinal':i+1,'created_at_ms':2000+i,'completed_at_ms':2000+i,
                    'parts':[{'id':f'{name}-part-{i}','kind':'text','ordinal':1,'state':'completed','content_text':f'HISTORY_{i:04d} '+('历史 wrap words '*7),'content_json':None,'content_digest':None,'created_at_ms':2000+i,'updated_at_ms':2000+i}]})
            command(self.sock, {'type':'import_conversation_transcript','input':{'import_id':name+'-archive','source':'external_archive','conversation_id':name,'expected_message_count':0,'imported_at_ms':3000,'messages':messages}})
    def open(self, session, label):
        case=self.root/label; case.mkdir()
        self.oracle=Oracle(self.binary.parent/'examples/terminal_acceptance_oracle')
        self.term=RealTerminal(self.binary,self.project,self.env,self.oracle,case,arguments=[str(self.binary),'--session',session,'--project',str(self.project),'--lang','en','--no-tools'])
        self.term.wait(lambda:'Context' in self.term.screen(),20,'composer')
    def close_terminal(self):
        if self.term:
            if not self.term.pty_closed: self.term.write(b"\x05\x15")
            self.term.close(); self.term=None
        if self.oracle: self.oracle.close(); self.oracle=None
    def cleanup(self):
        self.release.set()
        failures=[]
        for name,action in [('terminal',self.close_terminal),('service',lambda:stop_service(self.service) if self.service else None),('server',self.server.shutdown),('socket',self.server.server_close),('server_thread',lambda:self.thread.join(timeout=5)),('credentials',lambda:scrub_case_credentials(self.root))]:
            try: action()
            except Exception as e: failures.append({'stage':name,'error':type(e).__name__})
        if hashlib.sha256(self.binary.read_bytes()).hexdigest()!=self.report['binary_sha256'] or hashlib.sha256(self.service_binary.read_bytes()).hexdigest()!=self.report['service_sha256']:
            failures.append({'stage':'artifact_identity','error':'binary changed during scenario'})
        self.report['cleanup_failures']=failures
        if failures: self.report['passed']=False
        (self.root/'result.json').write_text(json.dumps(self.report,indent=2))
        (self.root/'requests.json').write_text(json.dumps(self.requests))
        self.assertFalse(failures)
    def test_long_history_reading_position_survives_restart(self):
        self.seed('long-history',320)
        self.open('long-history','before')
        offset=len(self.term.output)
        self.term.write(b'/history\r')
        self.term.write(b'HISTORY_READY')
        self.term.wait(lambda:self.view('long-history').get('draft')=='HISTORY_READY',10,'history replay returned to editor')
        self.term.write(b'\x15')
        self.term.wait(lambda:self.view('long-history').get('draft')=='',5,'clear readiness draft')
        for _ in range(100):
            frame=self.term.latest['frameId']
            self.term.write(b'\x1b[5~')
            self.term.wait(lambda:self.term.latest['frameId']>frame,5,'page up painted')
            ids=[int(x) for x in re.findall(r'HISTORY_(\d{4})',self.term.screen())]
            if ids and min(ids)<40: break
        self.assertTrue(ids and min(ids)<40, self.term.screen())
        marker=f'HISTORY_{min(ids):04d}'
        self.term.write('草稿保持'.encode())
        self.term.wait(lambda:'草稿保持' in self.term.screen() and self.view('long-history').get('draft')=='草稿保持',5,'draft')
        self.report['view_before']=self.view('long-history')
        marker=re.search(r'HISTORY_\d{4}',self.report['view_before']['viewport']['visible_line']).group()
        self.assertIn(marker,self.term.screen())
        self.report['marker']=marker
        self.term._signal_owned_group(signal.SIGHUP)
        self.term.wait(lambda:self.term.pty_closed,10,'closed')
        self.close_terminal()
        self.open('long-history','after')
        self.term.wait(lambda:marker in self.term.screen(),10,'old reading anchor restored beyond tail window')
        self.assertIn('草稿保持',self.term.screen())
        import fcntl, struct, termios
        frame=self.term.latest['frameId']
        self.oracle.query(operation='resize',rows=24,columns=60)
        fcntl.ioctl(self.term.master,termios.TIOCSWINSZ,struct.pack('HHHH',24,60,0,0))
        os.kill(self.term.pid,signal.SIGWINCH)
        self.term.wait(lambda:self.term.latest['frameId']>frame and marker in self.term.screen(),10,'reading anchor survives narrow reflow')
        self.assertIn('草稿保持',self.term.screen())
        self.assertEqual(self.query('SELECT count(*) FROM runs'),[(0,)])
        self.report.update(passed=True,marker=marker)
    def test_queued_input_does_not_cross_conversation(self):
        self.seed('queue-a',0); self.seed('queue-b',1)
        self.open('queue-a','queue')
        self.term.write(b'HOLD_A\r')
        self.term.wait(lambda:len(self.requests)==1,20,'physical request active')
        self.term.write(b'QUEUED_FOR_A\r')
        self.term.wait(lambda:bool(self.query("SELECT payload_json FROM run_events WHERE event_kind='input_queued'")),10,'queued input committed by Core')
        admissions=[json.loads(raw)['admission'] for raw, in self.query("SELECT payload_json FROM run_events WHERE event_kind='input_accepted'")]
        queued=[a for a in admissions if a['delivery']=='next_turn']
        self.assertEqual(len(queued),1)
        self.report['queued_admission']=queued[0]
        self.term.write(b'/resume\r')
        self.term.wait(lambda:b'Recent sessions' in self.term.output,10,'picker')
        self.term.write(b'\x1b[B\r')
        self.term.wait(lambda:'HISTORY_0000' in self.term.screen(),15,'second session')
        self.release.set()
        self.term.write(b'INPUT_B\r')
        self.term.wait(lambda:'ANSWER_B' in self.term.screen(),25,'B answer')
        self.assertNotIn('ANSWER_A', self.term.screen(), 'late A response leaked into B')
        b_requests=[r for r in self.requests if 'INPUT_B' in json.dumps(r)]
        self.assertTrue(b_requests)
        self.assertTrue(all('QUEUED_FOR_A' not in json.dumps(r) and 'HOLD_A' not in json.dumps(r) for r in b_requests))
        self.assertEqual(self.query("SELECT count(*) FROM messages WHERE conversation_id='queue-b' AND role='user'"),[(2,)])
        self.term.wait(lambda:self.query("SELECT status FROM runs WHERE conversation_id='queue-b'")==[('sealed',)],20,'B sealed')
        self.term.write(b'/resume\r')
        offset=len(self.term.output)
        self.term.wait(lambda:b'Recent sessions' in self.term.output[offset:],10,'return picker')
        self.term.write(b'\x1b[B\r')
        self.term.wait(lambda:any('QUEUED_FOR_A' in json.dumps(r) for r in self.requests[1:]),25,'A queued input resumed')
        self.term.wait(lambda:len(self.query("SELECT id FROM runs WHERE conversation_id='queue-a' AND status='sealed'"))==2,25,'A successor sealed')
        self.assertEqual(self.query("SELECT count(*) FROM messages WHERE conversation_id='queue-a' AND role='user'"),[(2,)])
        self.assertTrue(all('INPUT_B' not in json.dumps(r) for r in self.requests if 'QUEUED_FOR_A' in json.dumps(r)))
        applications=[a for raw, in self.query("SELECT payload_json FROM run_events WHERE event_kind='input_applied'") for a in json.loads(raw)['applications']]
        self.assertEqual(sum(a['admission_id']==queued[0]['admission_id'] for a in applications),1)
        self.report.update(passed=True,physical_request_count=len(self.requests))
    def test_remote_host_lost_receipt_reconciles_without_redispatch(self):
        import socket
        import urllib.request
        from test_real_submission_journal import Host
        self.seed('remote-fault',0)
        host=Host.__new__(Host)
        host.directory=self.root; host.binary=self.binary; host.http=urllib.request.build_opener(urllib.request.ProxyHandler({}))
        with socket.socket() as reservation:
            reservation.bind(('127.0.0.1',0)); port=reservation.getsockname()[1]
        host.base=f'http://127.0.0.1:{port}'
        with (self.root/'web.log').open('w') as log:
            process=subprocess.Popen([str(self.binary),'web','--project',str(self.project),'--host','127.0.0.1','--port',str(port),'--web-root',str(self.project)],env=self.env,stdout=log,stderr=log)
            try:
                deadline=time.monotonic()+30
                while True:
                    self.assertIsNone(process.poll())
                    try: host.host_id=host.json('/api/health')['host']['id']; break
                    except (OSError,RuntimeError):
                        self.assertLess(time.monotonic(),deadline); time.sleep(.1)
                import uuid
                submission=str(uuid.uuid4())
                identity=dict(source_host_id=host.host_id,client_id='fault-acceptance',conversation_id='remote-fault',submission_id=submission,user_message_id=submission+':user',assistant_message_id=submission+':assistant')
                plan=host.operation('/api/agent/tool-plan/prepare',dict(expected_host_id=host.host_id,client_id=identity['client_id'],session_id='remote-fault'))
                body=dict(conversationId='remote-fault',expected_host_id=host.host_id,client_id=identity['client_id'],submission_id=submission,user_message_id=identity['user_message_id'],assistant_message_id=identity['assistant_message_id'],execution_backend='nola',tool_plan_ref=plan['reference'],messages=[dict(role='user',content='INPUT_B')],max_iterations=3,expected_configuration_revision=2)
                prepared=host.operation('/api/chat/agent/v2/submission',dict(operation='prepare',identity=identity,request=body))
                body['request_digest']=prepared['request_digest']
                original_request=host.request
                def diagnostic_request(path, body=None, timeout=30):
                    request=urllib.request.Request(host.base+path, data=None if body is None else json.dumps(body).encode(), headers={'Content-Type':'application/json'})
                    try: return host.http.open(request,timeout=timeout)
                    except urllib.error.HTTPError as error:
                        detail=error.read().decode(); (self.root/'http-error.json').write_text(detail)
                        raise RuntimeError(f'HTTP {error.code}: {path}')
                host.request=diagnostic_request
                delivered=threading.Event(); outcomes=[]; drops_before_dispatch=[True]
                class DropReceipt(http.server.BaseHTTPRequestHandler):
                    def log_message(self,*_): pass
                    def do_POST(self):
                        request=json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                        # Execute through the real remote Host. Drop only its
                        # completed HTTP response, never synthesize a receipt.
                        try:
                            if drops_before_dispatch[0]:
                                drops_before_dispatch[0]=False
                            else:
                                outcomes.append(host.stream(request))
                        except Exception as error: outcomes.append(str(error))
                        finally:
                            delivered.set()
                            self.close_connection=True
                            self.connection.shutdown(socket.SHUT_RDWR)
                            self.connection.close()
                relay=http.server.ThreadingHTTPServer(('127.0.0.1',0),DropReceipt)
                worker=threading.Thread(target=relay.serve_forever,daemon=True); worker.start()
                try:
                    request=urllib.request.Request(f'http://127.0.0.1:{relay.server_port}/lost',data=json.dumps(body).encode(),headers={'Content-Type':'application/json'})
                    with self.assertRaises((OSError, http.client.HTTPException)):
                        urllib.request.urlopen(request,timeout=45)
                    self.assertTrue(delivered.wait(5))
                    self.assertEqual(self.query('SELECT count(*) FROM runs'),[(0,)])
                    self.assertFalse(self.requests, 'pre-dispatch disconnect started inference')
                    pending=host.operation('/api/chat/agent/v2/submission',dict(operation='observe',identity=identity,request_digest=body['request_digest']))
                    self.assertIsNone(pending, 'unsent request fabricated a source admission')
                    delivered.clear()
                    with self.assertRaises((OSError, http.client.HTTPException)):
                        urllib.request.urlopen(request,timeout=45)
                    self.assertTrue(delivered.wait(5))
                    (self.root/'remote-events.json').write_text(json.dumps(outcomes))
                    self.assertIsInstance(outcomes[0],list,outcomes)
                    seal,manifest=host.completed(identity,body['request_digest'])
                    self.assertEqual(host.text(manifest).strip(),'ANSWER_B')
                    calls=len(self.requests)
                    host.attempt_frontier=lambda identity: self.query('SELECT id,record_json FROM run_attempts WHERE run_id=? ORDER BY id',(identity['submission_id'],))
                    frontier=host.attempt_frontier(identity)
                    for restart in (False, True):
                        if restart:
                            process.terminate(); process.wait(timeout=10)
                            process=subprocess.Popen([str(self.binary),'web','--project',str(self.project),'--host','127.0.0.1','--port',str(port),'--web-root',str(self.project)],env=self.env,stdout=log,stderr=log)
                            deadline=time.monotonic()+30
                            while True:
                                self.assertIsNone(process.poll())
                                try:
                                    self.assertEqual(host.json('/api/health')['host']['id'],host.host_id)
                                    break
                                except OSError:
                                    self.assertLess(time.monotonic(),deadline); time.sleep(.1)
                        duplicate=host.stream(body)
                        self.assertTrue(duplicate)
                        self.assertTrue(all(e.get('type')=='submission_receipt' for e in duplicate))
                        self.assertEqual(host.completed(identity,body['request_digest']),(seal,manifest))
                        self.assertEqual(host.attempt_frontier(identity),frontier)
                        self.assertEqual(len(self.requests),calls)
                    self.assertEqual(self.query('SELECT count(*) FROM runs'),[(1,)])
                    self.report.update(passed=True,remote_request_identity=identity,physical_request_count=calls,pre_dispatch_disconnect=True,post_dispatch_receipt_lost=True,cold_host_reconciliation=True)
                finally:
                    relay.shutdown(); relay.server_close(); worker.join(timeout=5)
            finally:
                process.terminate()
                try: process.wait(timeout=10)
                except subprocess.TimeoutExpired: process.kill(); process.wait(timeout=10)

    def test_remote_partial_stream_is_not_sealed_as_answered(self):
        from canonical_run_audit import audit_runs
        self.seed('partial-stream',0)
        self.truncate_stream=True
        with (self.root/'stream-failure.ndjson').open('w') as log:
            result=subprocess.run([str(self.binary),'Read the remote response.','--session','partial-stream','--project',str(self.project),'--json','--no-tools','--max-iterations','1'],env=self.env,stdout=log,stderr=log,timeout=40)
        self.assertTrue(self.requests)
        with closing(sqlite3.connect((self.root/'nola.db').as_uri()+'?mode=ro',uri=True)) as db:
            audit=audit_runs(db,'partial-stream')
        self.assertEqual(audit['runCount'],1)
        self.assertEqual(audit['outcomes'], ['incomplete'], audit)
        self.assertEqual(len(self.requests), 2, 'bounded continuation must stop after a second truncated response')
        self.assertNotIn('application_rejected', (self.root/'stream-failure.ndjson').read_text())
        self.report.update(passed=True,stream_audit=audit,physical_request_count=len(self.requests),exit_code=result.returncode)
