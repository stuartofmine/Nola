#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Real selected-model native/Web acceptance in owned runtime stores.

A local relay holds the actual credential; clients only receive a disposable
non-secret bearer value. No generation responses are fabricated.
"""
import argparse
import contextlib
from http.server import ThreadingHTTPServer
import json
import os
import socket
import signal
import sqlite3
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import time
from urllib.parse import urlsplit

from e2e.provider_probe import Handler, Probe
from provider_stream_acceptance import require_text_stream
from test_real_cli_long_tasks import selected_route, fixture, apply_config
from canonical_run_audit import audit_runs

ROOT = Path(__file__).resolve().parent.parent


@contextlib.contextmanager
def relay(upstream, model, secret, root):
    expected = urlsplit(upstream).path.rstrip('/') + '/responses'
    request_state = threading.local()
    forwarded_inputs = []
    class SelectedProbe(Probe):
        def response_override(self, path, request, headers):
            if path != expected or request.get('model') != model or not request.get('input'):
                return 403, b'{"error":{"message":"Selected acceptance generation route only"}}'
            request_state.input = request.get('input')
            return None
    class AuthorizedHandler(Handler):
        def do_GET(self):
            if self.path == '/__nola_test/forwarded':
                body = json.dumps(forwarded_inputs).encode()
                self.send_response(200); self.send_header('Content-Type', 'application/json')
                self.send_header('Content-Length', str(len(body))); self.end_headers(); self.wfile.write(body)
            else:
                super().do_GET()
        def send_response(self, code, message=None):
            # Handler calls this after actual upstream response headers arrive.
            if code == 200 and getattr(request_state, 'input', None):
                forwarded_inputs.append(request_state.input)
            super().send_response(code, message)
        def do_POST(self):
            request_state.input = None
            if 'Authorization' in self.headers:
                del self.headers['Authorization']
            self.headers['Authorization'] = 'Bearer ' + secret
            super().do_POST()
    server = ThreadingHTTPServer(('127.0.0.1', 0), AuthorizedHandler)
    server.probe = SelectedProbe(upstream, root / 'provider.ndjson', 240, capture_last_user_message=True)
    thread = threading.Thread(target=server.serve_forever, daemon=True); thread.start()
    try:
        yield f'http://127.0.0.1:{server.server_port}{urlsplit(upstream).path.rstrip("/")}'
    finally:
        server.shutdown(); server.server_close(); thread.join(timeout=5)


def run(command, env, root, name, timeout=1800, cwd=ROOT):
    print(name, 'started', flush=True)
    with (root / (name + '.log')).open('w') as output:
        process = subprocess.Popen(command, env=env, cwd=cwd, stdout=output,
            stderr=subprocess.STDOUT, start_new_session=True)
        try:
            code = process.wait(timeout=timeout)
        except (subprocess.TimeoutExpired, KeyboardInterrupt):
            os.killpg(process.pid, signal.SIGTERM)
            try:
                process.wait(timeout=20)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=5)
            raise
    print(name, 'exit', code, flush=True)
    if code:
        raise RuntimeError(f'{name} failed; see {root / (name + ".log")}')


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--platform', choices=['mac', 'ios', 'android', 'web', 'all'], default='all')
    parser.add_argument('--config', type=Path, default=Path.home()/'.nola/config.json')
    parser.add_argument('--ios-case', choices=['all', 'both', 'draft', 'lifecycle', 'setup', 'keyboard'], default='all')
    args = parser.parse_args()
    if args.platform == 'all':
        # Native UI suites run sequentially; each subprocess owns its relay,
        # isolated stores, bounded children and evidence directory.
        results = {}
        for platform in ('web', 'mac', 'ios', 'android'):
            result = subprocess.run([sys.executable, str(Path(__file__).resolve()),
                '--platform', platform, '--config', str(args.config), '--ios-case', args.ios_case], cwd=ROOT)
            results[platform] = result.returncode
        print(json.dumps({'platform_exit_codes': results}), flush=True)
        return 0 if all(code == 0 for code in results.values()) else 1
    upstream, model, secret = selected_route(args.config)
    root = Path(tempfile.mkdtemp(prefix='nola-cross-real-' + args.platform + '-')); root.chmod(0o700)
    print('Evidence:', root, flush=True)
    env = {k:v for k,v in os.environ.items() if k in {'HOME','PATH','USER','LOGNAME','SHELL','TMPDIR','LANG','TERM'} or k.startswith('LC_')}
    env.update(CARGO_BUILD_JOBS='2', NOLA_ACCEPTANCE_RELAY_KEY='owned-relay-no-provider-secret')
    report = {'platform': args.platform, 'model': model, 'passed': False}
    try:
        with relay(upstream, model, secret, root) as endpoint:
            config = root/'config.json'
            config.write_text(json.dumps({'llm': {'connections': [{'id':'acceptance','endpoint':endpoint,'protocol':'responses',
                'api_key_env':'NOLA_ACCEPTANCE_RELAY_KEY','models':[{'id':model}]}], 'default_model':{'connection':'acceptance','model':model}}}))
            env['NOLA_CONFIG_PATH'] = str(config)
            if args.platform == 'mac':
                env.update(NOLA_MAC_REAL_XCTEST_DERIVED_DATA='/tmp/nola-ui-polish-mac-build',
                    NOLA_MAC_REAL_MEMORY_PRESERVE_ARTIFACTS='1', NOLA_MAC_REAL_PRESERVE_DERIVED_DATA='1',
                    NOLA_MAC_REAL_ONLY_TESTING='NolaMacTests/MacRealProductUIEndToEndTests/testRealStreamingSurvivesConversationSwitchAndKeepsLatestVisible|||NolaMacTests/MacModelTestReasoningTierTests',
                    NOLA_MAC_REAL_EVIDENCE_PROFILE='product_ui', NOLA_MAC_REAL_EXPECTED_TEST_COUNT='4',
                    NOLA_MAC_REAL_MIN_SUCCESSFUL_RUNS='1')
                run(['bash', 'scripts/run_real_mac_xctests.sh'], env, root, 'mac-streaming')
            elif args.platform == 'ios':
                devices = json.loads(subprocess.check_output(['xcrun','simctl','list','devices','available','--json']))
                device = next(d['udid'] for runtime, values in devices['devices'].items() if '.iOS-' in runtime
                    for d in values if d.get('isAvailable'))
                env.update(INCLUDE_UI_TESTS='1', PARALLEL_TEST_WORKERS='1', SIMULATOR_SIGNING='adhoc',
                    DESTINATION='platform=iOS Simulator,id='+device, DERIVED_DATA='/tmp/nola-mobile-summary-ios',
                    RESULT_BUNDLE=str(root/'ios.xcresult'), XCODE_OPTS='-configuration Test -jobs 2 -collect-test-diagnostics never SWIFT_EMIT_LOC_STRINGS=NO')
                for key, value in {'NOLA_UI_REPRO_ENDPOINT':endpoint, 'NOLA_UI_REPRO_MODEL':model,
                    'NOLA_UI_REPRO_PROTOCOL':'responses'}.items():
                    env['TEST_RUNNER_'+key] = value
                cases = [('draft', 'testDraftKeyboardMenusAndColdRestoreRemainConversationOwned'),
                    ('lifecycle', 'testRealProviderConversationIsolationAndLongRunNavigation'),
                    ('setup', 'testRealModelSetupThenChat'),
                    ('keyboard', 'testRealKeyboardDismissalAndCommittedHistorySurviveTabNavigation')]
                for index, (name, method) in enumerate(cases):
                    if (args.ios_case == 'both' and name not in {'draft', 'lifecycle'}) or args.ios_case not in {'all', 'both', name}:
                        continue
                    result_path = root/(name+'.xcresult')
                    env['RESULT_BUNDLE'] = str(result_path)
                    # Seed only current model configuration in an owned Store;
                    # this scenario tests chat/draft lifecycle, not keyboard entry
                    # in the model setup form. Native UI still admits every Run.
                    case_root = root/('ios-'+name+'-store')
                    if name != 'setup':
                        project, preparation_env, configuration = fixture(case_root, model, endpoint)
                        apply_config(ROOT/'target/debug/nola', project, preparation_env, configuration, case_root)
                    else:
                        case_root.mkdir(parents=True)
                    for key, value in {
                        'NOLA_UI_REUSE_CONFIGURED_MODELS':'0' if name == 'setup' else '1',
                        'NOLA_UI_EXISTING_LOCAL_NAMESPACE':root.name+'-'+name,
                        'NOLA_UI_REAL_DATABASE_PATH':str(case_root/'nola.db'),
                        'NOLA_UI_REPRO_API_KEY':'',
                        'NOLA_UI_SETUP_EVIDENCE_ROOT':str(root),
                    }.items():
                        env['TEST_RUNNER_'+key] = value
                    if index and args.ios_case in {'all', 'both'}:
                        env['SKIP_BUILD'] = '1'
                    run(['bash','scripts/run_client_tests.sh',
                        '-only-testing:NolaUITests/IOSRealConversationLifecycleTests/'+method],
                        env, root, 'ios-'+name)
                    summary = json.loads(subprocess.check_output(['xcrun','xcresulttool','get','test-results','summary',
                        '--path',str(result_path),'--compact']))
                    if summary.get('passedTests') != 1 or summary.get('skippedTests') != 0 or summary.get('failedTests') != 0:
                        raise RuntimeError('The selected iOS real UI test must pass without skips')
                    (root/('ios-'+name+'-summary.json')).write_text(json.dumps(summary, indent=2))
                    with sqlite3.connect((case_root/'nola.db').as_uri()+'?mode=ro', uri=True) as database:
                        audits = [audit_runs(database, row[0]) for row in database.execute(
                            'SELECT DISTINCT conversation_id FROM runs WHERE kind = ?', ('conversation',))]
                    if not audits or not any('answered' in audit['outcomes'] for audit in audits):
                        raise RuntimeError('iOS acceptance requires an answered canonical Run with committed result and seal')
                    (root/('ios-'+name+'-canonical-audit.json')).write_text(json.dumps(audits, indent=2))
            elif args.platform == 'android':
                android(root, env, endpoint, model)
            else:
                web(root, env, endpoint, model)
        records = [json.loads(line) for line in (root/'provider.ndjson').read_text().splitlines()]
        real = [r for r in records if r.get('forwarded') is not False]
        report.update(provider_requests=len(real), successful_provider_requests=sum(r.get('status')==200 for r in real),
            observed_text_streams=require_text_stream(real), passed=True)
    except Exception as error:
        report['error'] = str(error)
    finally:
        (root/'receipt.json').write_text(json.dumps(report, indent=2))
        for path in root.rglob('*'):
            if path.is_file() and secret.encode() in path.read_bytes():
                raise RuntimeError('Credential found in evidence; do not publish')
    print(json.dumps(report), flush=True)
    return 0 if report['passed'] else 1


def android(root, env, endpoint, model):
    sdk = Path.home()/'Library/Android/sdk'
    adb = str(sdk/'platform-tools/adb')
    serial = 'emulator-5580'
    if serial in subprocess.check_output([adb,'devices'], text=True):
        raise RuntimeError('Owned emulator port is already occupied')
    log = (root/'emulator.log').open('w')
    emulator = subprocess.Popen([str(sdk/'emulator/emulator'), '-avd','nola_avd', '-read-only',
        '-no-snapshot', '-no-window', '-no-audio', '-no-boot-anim', '-port','5580', '-gpu','software'],
        env=env, stdout=log, stderr=subprocess.STDOUT)
    try:
        deadline = time.monotonic()+180
        while time.monotonic()<deadline:
            boot = subprocess.run([adb,'-s',serial,'shell','getprop','sys.boot_completed'], capture_output=True, text=True, timeout=10)
            if boot.stdout.strip()=='1':
                break
            if emulator.poll() is not None:
                raise RuntimeError('Owned read-only emulator exited')
            time.sleep(1)
        else:
            raise TimeoutError('Owned emulator did not boot')
        created = subprocess.check_output([adb,'-s',serial,'shell','pm','create-user','Nola Real Long Task'], text=True)
        user = created.strip().split()[-1]
        if not user.isdigit() or int(user)==0:
            raise RuntimeError('Could not create disposable Android user')
        env.update(NOLA_ANDROID_TEST_USER_ID=user, NOLA_ANDROID_REAL_SERIAL=serial, NOLA_ANDROID_JAVA_HOME='/Library/Java/JavaVirtualMachines/temurin-21.jdk/Contents/Home')
        run(['bash','scripts/test_real_android_chat_product_ui.sh'], env, root, 'android-streaming')
        # A second real UI scenario covers background/return, Stop, next turn and activity recreation.
        command = [adb,'-s',serial,'shell','am','switch-user',user]
        subprocess.run(command,check=True,stdout=subprocess.DEVNULL)
        fixture_path = root/'android-fixture.json'
        fixture_path.write_text(json.dumps({'endpoint':endpoint,'apiKey':'owned-relay-no-provider-secret','model':model,'protocol':'responses'}))
        port = str(urlsplit(endpoint).port)
        subprocess.run([adb,'-s',serial,'reverse','tcp:'+port,'tcp:'+port],check=True,stdout=subprocess.DEVNULL)
        with fixture_path.open('r') as content:
            subprocess.run([adb,'-s',serial,'shell',f"run-as com.nola --user {user} sh -c 'cat > files/nola-real-llm-e2e.json'"],stdin=content,check=True)
        run([adb,'-s',serial,'shell','am','instrument','--user',user,'-w','-r','-e','isolatedUserId',user,
            '-e','class','com.nola.ui.screen.chat.RealAndroidChatProductUiEndToEndTest#realStreamingStopAllowsNextTurnAndActivityRestore,com.nola.ui.screen.chat.MessageCopyActionInteractionTest',
            'com.nola.test/androidx.test.runner.AndroidJUnitRunner'],env,root,'android-stop-restore',timeout=600)
        if 'OK (2 tests)' not in (root/'android-stop-restore.log').read_text():
            raise RuntimeError('Android stop/restore instrumentation did not pass')
    finally:
        if 'user' in locals():
            # Keep failures and the second lifecycle scenario, not only the
            # first shell wrapper's happy-path artifacts.
            evidence = root/'android-lifecycle'
            evidence.mkdir(exist_ok=True)
            with (evidence/'logcat.txt').open('wb') as output:
                subprocess.run([adb,'-s',serial,'logcat','-d'],stdout=output,stderr=subprocess.DEVNULL,timeout=15)
            subprocess.run([adb,'-s',serial,'shell','am','force-stop','--user',user,'com.nola'],
                stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,timeout=10)
            for name in ['real-android-stop-background.json','real-android-stop.png','real-android-stop-window-metrics.json',
                'real-android-stop-accessibility.txt','real-model-selection-failure.txt']:
                with (evidence/name).open('wb') as output:
                    captured = subprocess.run([adb,'-s',serial,'exec-out','run-as','com.nola','--user',user,
                        'cat','files/'+name],stdout=output,stderr=subprocess.DEVNULL,timeout=10)
                if captured.returncode:
                    (evidence/name).rename(evidence/(name+'.missing.txt'))
            for suffix in ['', '-wal', '-shm']:
                with (evidence/('nola-product.db'+suffix)).open('wb') as output:
                    subprocess.run([adb,'-s',serial,'exec-out','run-as','com.nola','--user',user,
                        'cat',f'/data/user/{user}/com.nola/no_backup/nola-product.db'+suffix],
                        stdout=output,stderr=subprocess.DEVNULL,timeout=10)
        subprocess.run([adb,'-s',serial,'emu','kill'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,timeout=10)
        try:
            emulator.wait(timeout=20)
        except subprocess.TimeoutExpired:
            emulator.terminate(); emulator.wait(timeout=10)
        log.close()


def web(root, env, endpoint, model):
    binary = ROOT/'target/debug/nola'
    project, host_env, config = fixture(root/'store', model, endpoint)
    apply_config(binary, project, host_env, config, root/'store')
    (project/'long_job.py').write_text("import json,time\nfrom pathlib import Path\nwith Path('starts.txt').open('a') as f: f.write('start\\n')\nfor i in range(1,31):\n time.sleep(1)\n with Path('progress.txt').open('a') as f: f.write(str(i)+'\\n')\nPath('result.json').write_text(json.dumps({'total':465,'steps':30}))\n")
    with socket.socket() as listener:
        listener.bind(('127.0.0.1',0)); port = listener.getsockname()[1]
    host_log = (root/'host.log').open('w')
    host = subprocess.Popen([str(binary),'web','--host','127.0.0.1','--port',str(port),
        '--web-root',str(ROOT/'web'),'--project',str(project),'--allow-dir',str(project)],
        cwd=project, env=host_env, stdout=host_log, stderr=subprocess.STDOUT)
    try:
        import urllib.request
        url = f'http://127.0.0.1:{port}'
        deadline = time.monotonic()+30
        while time.monotonic()<deadline:
            try:
                urllib.request.urlopen(url+'/login',timeout=2).close(); break
            except OSError:
                if host.poll() is not None: raise RuntimeError('Isolated Web Host exited')
                time.sleep(.2)
        else: raise TimeoutError('Isolated Web Host did not start')
        env.update(NOLA_LONG_WEB_URL=url,NOLA_LONG_WEB_OUT=str(root))
        run(['npm','run','build'], env, root, 'web-build',cwd=ROOT/'web/frontend')
        run(['node','web/playwright_runtime/real_cross_platform_long_task.cjs'],env,root,'web-long-task',timeout=600)
        run(['node','web/playwright_runtime/ui_ergonomics_regression.cjs'],env,root,'web-ergonomics',timeout=300)
    finally:
        host.terminate()
        try: host.wait(timeout=15)
        except subprocess.TimeoutExpired: host.kill(); host.wait(timeout=5)
        host_log.close()


if __name__ == '__main__':
    raise SystemExit(main())
