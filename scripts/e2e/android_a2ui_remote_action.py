#!/usr/bin/env python3
"""Opt-in real Android UI -> paired isolated Host -> model -> form action cycle."""
import argparse
import http.cookiejar
import json
import os
from pathlib import Path
import subprocess
import tempfile
import urllib.request
from urllib.parse import urlsplit


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--serial', required=True)
    parser.add_argument('--state', type=Path, required=True, help='Explicit isolated source Host state; never select an old Host implicitly')
    parser.add_argument('--skip-build', action='store_true')
    parser.add_argument('--resume-conversation', help='Continue the retained source conversation without resending its prompt')
    args = parser.parse_args()
    state = json.loads(args.state.read_text())
    root = Path(state['root']).resolve(strict=True)
    parsed = urlsplit(state['url'])
    assert root.name.startswith('nola-a2ui-real-') and parsed.hostname == '127.0.0.1' and parsed.scheme == 'http'
    assert args.serial.startswith('emulator-')
    token = (root/'home/.nola/web/mobile-access-token').read_text().strip()
    base = f'http://127.0.0.1:{parsed.port}'
    opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))
    def request(path, body=None):
        req = urllib.request.Request(base+path, data=None if body is None else json.dumps(body).encode(), headers={'Content-Type':'application/json'})
        with opener.open(req, timeout=30) as response:
            return json.load(response)
    request('/api/web/login', {'token':token})
    host = request('/api/web/session')['hostId']
    repo = Path(__file__).resolve().parents[2]
    adb = Path(os.environ.get('ANDROID_HOME', str(Path.home()/'Library/Android/sdk')))/'platform-tools/adb'
    evidence = Path(tempfile.mkdtemp(prefix='nola-a2ui-android-remote-action-')); evidence.chmod(0o700)
    def device(*command, data=None, timeout=120):
        result = subprocess.run([str(adb),'-s',args.serial,*command], input=data, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=timeout)
        if result.returncode:
            raise RuntimeError('Device command failed: '+result.stdout.decode(errors='replace').replace(token,'[redacted]')[-1000:])
        return result.stdout
    assert device('emu','avd','name').decode().splitlines()[0].startswith('nola_a2ui_isolated_')
    if not args.skip_build:
        with (evidence/'build.log').open('wb') as log:
            subprocess.run(['./gradlew',':app:assembleDebug',':app:assembleDebugAndroidTest','-x','buildRustCore','--max-workers=2'], cwd=repo/'apps/android',stdout=log,stderr=subprocess.STDOUT,check=True)
    device('install','-r',str(repo/'apps/android/app/build/outputs/apk/debug/app-debug.apk'))
    device('install','-r',str(repo/'apps/android/app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk'))
    port = f'tcp:{parsed.port}'
    target = 'com.nola.ui.generative.GenerativeUiRealModelEndToEndTest'
    print(json.dumps({'stage':'starting_remote_action','evidence':str(evidence),'sourceHost':host}),flush=True)
    try:
        device('reverse',port,port)
        device('shell','run-as','com.nola','mkdir','-p','files')
        fixture = {'disposableFixture':True,'remoteHost':True,'url':base,'token':token,'hostId':host,
                   'model':'web-host','displayModel':'Nola Web Host'}
        if args.resume_conversation:
            fixture['resumeConversation'] = args.resume_conversation
        device('shell','run-as','com.nola','sh','-c',"'cat > files/a2ui-real-model-fixture.json'",data=json.dumps(fixture).encode())
        output = device('shell','am','instrument','-w','-r','-e','disposableA2uiFixture','true','-e','class',
                        target+'#realProviderFormEditsSubmitsUpdatesAndReopens','com.nola.test/androidx.test.runner.AndroidJUnitRunner',timeout=650)
        (evidence/'instrumentation.log').write_bytes(output.replace(token.encode(),b'[redacted]'))
        assert b'OK (1 test)' in output, 'Remote action acceptance failed; inspect '+str(evidence)
        result = json.loads(device('exec-out','run-as','com.nola','cat','files/a2ui-real-model-result.json'))
        assert result['passed'] is True and result['realSourceHost'] == host and result['sourceAdmissions']
        (evidence/'result.json').write_text(json.dumps(result,indent=2))
        print(json.dumps({'passed':True,'evidence':str(evidence)}))
    finally:
        try:
            device('shell','am','instrument','-w','-r','-e','disposableA2uiFixture','true','-e','class',
                   target+'#clearFixtureCredential','com.nola.test/androidx.test.runner.AndroidJUnitRunner',timeout=60)
        finally:
            device('shell','run-as','com.nola','rm','-f','files/a2ui-real-model-fixture.json')
            device('reverse','--remove',port)

if __name__ == '__main__':
    main()
