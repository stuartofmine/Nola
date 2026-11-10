#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Real Android UI -> paired Host -> selected model -> long physical task.

Requires an explicitly selected emulator and disposable secondary Android user.
The side-channel oracle only reads the owned Host's actual files and Store.
"""
import argparse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import secrets
import signal
import socket
import sqlite3
import subprocess
import tempfile
import threading
import time
from urllib.request import urlopen

from canonical_run_audit import audit_runs
from test_real_cli_long_tasks import selected_route, observed_provider, fixture, apply_config
from test_real_cli_hundreds_steps import provider_totals
from test_real_remote_host_long_tasks import committed_tools

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--serial',required=True)
    parser.add_argument('--user',type=int,required=True)
    parser.add_argument('--avd',required=True)
    parser.add_argument('--config',type=Path,default=Path.home()/'.nola/config.json')
    parser.add_argument('--steps',type=int,default=120)
    parser.add_argument('--skip-build',action='store_true')
    args=parser.parse_args()
    if not args.serial.startswith('emulator-') or args.user<=0:parser.error('Disposable emulator secondary user required')
    if not 90<=args.steps<=180:parser.error('Use 90–180 physical one-second steps')
    sdk=Path(os.environ.get('ANDROID_SDK_ROOT',str(Path.home()/'Library/Android/sdk')))
    adb=str(sdk/'platform-tools/adb')
    root=Path(tempfile.mkdtemp(prefix='nola-android-remote-long-'));root.chmod(0o700)
    print(json.dumps({'stage':'started','evidence':str(root),'steps':args.steps}),flush=True)
    upstream,model,secret=selected_route(args.config)
    host=None;oracle=None;reverses=[];staged=False
    receipt={'passed':False,'steps':args.steps,'realModel':True,'realHost':True}

    def device(*command,data=None,timeout=120):
        result=subprocess.run([adb,'-s',args.serial,*map(str,command)],input=data,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,timeout=timeout)
        if result.returncode:raise RuntimeError('Owned emulator operation failed; '+result.stdout.decode(errors='replace')[-500:])
        return result.stdout

    def instrument(method,timeout=360):
        result=device('shell','am','instrument','--user',args.user,'-w','-r','-e','isolatedUserId',args.user,
            '-e','realRemoteLong','1','-e','class','com.nola.ui.screen.chat.RealAndroidRemoteLongTaskTest#'+method,
            'com.nola.test/androidx.test.runner.AndroidJUnitRunner',timeout=timeout)
        (root/(method+'.log')).write_bytes(result)
        assert b'OK (1 test)' in result and b'INSTRUMENTATION_STATUS_CODE: -3' not in result, method+' failed or skipped'

    try:
        assert device('emu','avd','name').decode().splitlines()[0].strip()==args.avd,'Unexpected AVD'
        assert device('shell','am','get-current-user').strip()==str(args.user).encode(),'Activate the declared test user first'
        if not args.skip_build:
            env=dict(os.environ)
            env['JAVA_HOME']=env.get('NOLA_ANDROID_JAVA_HOME',env.get('JAVA_HOME','/Library/Java/JavaVirtualMachines/temurin-21.jdk/Contents/Home'))
            with (root/'build.log').open('w') as log:
                subprocess.run([str(ROOT/'android/gradlew'),'-p',str(ROOT/'android'),':app:assembleDebug',':app:assembleDebugAndroidTest',
                    '--no-daemon','--max-workers=2'],env=env,stdout=log,stderr=subprocess.STDOUT,check=True)
        device('install','--user',args.user,'-r',ROOT/'android/app/build/outputs/apk/debug/app-debug.apk')
        device('install','--user',args.user,'-r',ROOT/'android/app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk')
        with observed_provider(upstream,model,secret,root/'provider.ndjson') as endpoint:
            project,env,config=fixture(root/'store',model,endpoint,profile='cautious')
            apply_config(ROOT/'target/debug/nola',project,env,config,root/'store')
            nonce=secrets.token_hex(16)
            program=("from pathlib import Path\nimport time,json\nwith Path('starts.txt').open('a') as f:f.write('start\\n')\n"
                +f"for i in range(1,{args.steps+1}):\n with Path('progress.txt').open('a') as f:f.write(str(i)+'\\n')\n time.sleep(1)\n"
                +f"Path('result.json').write_text(json.dumps({{'nonce':'{nonce}','steps':{args.steps},'total':{args.steps*(args.steps+1)//2}}}))\n")
            (project/'remote_job.py').write_text(program)
            with socket.socket() as sock:sock.bind(('127.0.0.1',0));port=sock.getsockname()[1]
            log=(root/'host.log').open('w')
            host=subprocess.Popen([str(ROOT/'target/debug/nola'),'web','--host','127.0.0.1','--port',str(port),
                '--project',str(project),'--allow-dir',str(project),'--web-root',str(ROOT/'web'),'--no-skills'],cwd=project,
                env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
            url=f'http://127.0.0.1:{port}'
            deadline=time.monotonic()+30
            tokenFile=root/'store/home/.nola/web/mobile-access-token'
            while time.monotonic()<deadline:
                try:
                    urlopen(url+'/login',timeout=2).close()
                    if tokenFile.exists():break
                except OSError:pass
                assert host.poll() is None,'Owned Host exited';time.sleep(.1)
            else:raise TimeoutError('Host startup')
            database=root/'store/nola.db'
            # Use the authenticated product endpoint to read the Host identity,
            # not a guessed database format or a model-generated identity.
            from http.cookiejar import CookieJar
            from urllib.request import build_opener,HTTPCookieProcessor,Request
            opener=build_opener(HTTPCookieProcessor(CookieJar()))
            with opener.open(Request(url+'/api/web/login',data=json.dumps({'token':tokenFile.read_text().strip()}).encode(),headers={'Content-Type':'application/json'})) as response:assert response.status==200
            with opener.open(url+'/api/web/session') as response:hostId=json.load(response)['hostId']
            expected={'nonce':nonce,'steps':args.steps,'total':args.steps*(args.steps+1)//2}

            def actual_state():
                ticks=(project/'progress.txt').read_text().splitlines() if (project/'progress.txt').exists() else []
                starts=(project/'starts.txt').read_text().splitlines() if (project/'starts.txt').exists() else []
                with sqlite3.connect(database.as_uri()+'?mode=ro',uri=True) as db:
                    rows=db.execute("SELECT r.id,r.conversation_id,r.assistant_message_id,r.status,json_extract(v.result_json,'$.value.kind') FROM runs r LEFT JOIN run_results v ON v.run_id=r.id WHERE kind='conversation' ORDER BY r.created_at_ms").fetchall()
                row=rows[0] if rows else [None]*5
                try:valid=json.loads((project/'result.json').read_text())==expected
                except (OSError,ValueError):valid=False
                return dict(ticks=len(ticks),starts=len(starts),runCount=len(rows),runId=row[0],conversationId=row[1],assistantMessageId=row[2],runStatus=row[3],outcome=row[4],resultValid=valid)

            class Oracle(BaseHTTPRequestHandler):
                def do_GET(self):
                    if self.path!='/state':self.send_error(404);return
                    body=json.dumps(actual_state()).encode();self.send_response(200);self.send_header('Content-Type','application/json');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
                def log_message(self,*unused):pass
            oracle=ThreadingHTTPServer(('127.0.0.1',0),Oracle)
            threading.Thread(target=oracle.serve_forever,daemon=True).start()
            for localPort in (port,oracle.server_port):
                rule=f'tcp:{localPort}';device('reverse',rule,rule);reverses.append(rule)
            setup={'url':url,'token':tokenFile.read_text().strip(),'hostId':hostId,'steps':args.steps,'project':str(project.resolve()),
                'oracle':f'http://127.0.0.1:{oracle.server_port}/state',
                'prompt':'请实际执行隔离项目中的 python3 remote_job.py，前台执行，timeout_sec=240。等待它真实完成后读取 result.json 并核验实际结果，最终以 ANDROID_REMOTE_LONG_DONE 开头报告。命令只启动一次，不要修改脚本，不要联网、派生助手或改用别的执行方式。'}
            device('shell','run-as','com.nola','--user',args.user,'mkdir','-p','files')
            device('shell','run-as','com.nola','--user',args.user,'sh','-c',"'cat > files/remote-long-fixture.json'",data=json.dumps(setup).encode());staged=True
            print(json.dumps({'stage':'native-submit-and-detach'}),flush=True)
            instrument('submitRealLongTaskThenDetachClient')
            before=actual_state();assert before['runStatus']=='running' and before['starts']==1
            device('shell','am','force-stop','--user',args.user,'com.nola')
            time.sleep(5);detached=actual_state()
            assert detached['runId']==before['runId'] and detached['runStatus']=='running' and detached['ticks']>before['ticks']
            (root/'detached.json').write_text(json.dumps(detached,indent=2))
            print(json.dumps({'stage':'native-cold-client-restore','ticks':detached['ticks']}),flush=True)
            instrument('coldClientObservesSameRunAndCompletedResult')
            final=actual_state();assert final['runId']==before['runId'] and final['runCount']==1 and final['resultValid'] and final['starts']==1
            assert (project/'progress.txt').read_text().splitlines()==[str(i) for i in range(1,args.steps+1)]
            assert (project/'remote_job.py').read_text()==program,'Input script was modified'
            tools=committed_tools(database,final['conversationId'])
            jobs=[t for t in tools if t['tool']=='bash' and t['arguments'].get('command')=='python3 remote_job.py']
            assert len(jobs)==1 and jobs[0]['state']=='completed','Physical job must complete exactly once'
            assert not any(t['tool']=='bash' and t not in jobs and t['arguments'].get('operation_kind')!='inspect'
                for t in tools),'Unexpected additional mutating command'
            with sqlite3.connect(database.as_uri()+'?mode=ro',uri=True) as db:receipt['canonical']=audit_runs(db,final['conversationId'])
            assert receipt['canonical']=={'runCount':1,'outcomes':['answered']}
            receipt.update(passed=True,final=final,provider=provider_totals(root/'provider.ndjson'))
    except Exception as error:receipt['error']=str(error)
    finally:
        for name in ['remote-long-running.png','remote-long-permission-review.png','remote-long-restored-result.png','remote-long-restored-tree.txt','remote-long-original.json','remote-long-detached.json','remote-long-result.json']:
            try:
                artifact=device('exec-out','run-as','com.nola','--user',args.user,'cat','files/'+name)
                if name.endswith('.png') and not artifact.startswith(b'\x89PNG\r\n\x1a\n'):continue
                if name.endswith('.json'):json.loads(artifact)
                (root/name).write_bytes(artifact)
            except Exception:pass
        if staged:
            try:instrument('clearOwnedHostFixture',60)
            except Exception:pass
        for rule in reverses:
            try:device('reverse','--remove',rule)
            except Exception:pass
        if oracle is not None:oracle.shutdown();oracle.server_close()
        if host is not None and host.poll() is None:
            os.killpg(host.pid,signal.SIGTERM)
            try:host.wait(timeout=8)
            except subprocess.TimeoutExpired:os.killpg(host.pid,signal.SIGKILL);host.wait(timeout=5)
        if 'log' in locals():log.close()
        (root/'receipt.json').write_text(json.dumps(receipt,indent=2))
    print(json.dumps({'passed':receipt['passed'],'evidence':str(root),'error':receipt.get('error')}),flush=True)
    return 0 if receipt['passed'] else 1


if __name__=='__main__':raise SystemExit(main())
