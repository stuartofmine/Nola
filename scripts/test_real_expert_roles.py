#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Real Responses provider -> isolated Main Agent -> leaf expert acceptance.

Uses the configured connection in memory through a restricted loopback relay.
No production Store writes, credential changes, browser input or mocked replies.
This is a live, potentially billable test, not a UI or all-provider certificate.
This suite verifies Main-only dispatch, expert effort, persistent history and
clarification via Main; it does not certify artifact Skill execution.
"""
import argparse
import http.cookiejar
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
from urllib.error import HTTPError, URLError
from urllib.request import HTTPCookieProcessor, Request, build_opener

from test_real_cli_long_tasks import selected_route, observed_provider, fixture
from test_real_remote_host_long_tasks import committed_tools
from real_e2e_events import committed_journal_parts

ROOT = Path(__file__).resolve().parents[1]


def answer_text(database, run_id):
    with sqlite3.connect(database.as_uri()+'?mode=ro', uri=True) as db:
        headers = [json.loads(row[0]) for row in db.execute('''SELECT r.header_json FROM assistant_journal_records r
            WHERE json_extract(r.owner,'$.run_id')=?
            AND r.sequence=(SELECT max(sequence) FROM assistant_journal_records WHERE owner=r.owner)''', (run_id,))]
    events = [{'type': 'assistant_journal_committed_v1', 'through': {key: header[key]
        for key in ('identity', 'sequence', 'record_digest')}} for header in headers]
    return '\n'.join(part['bodies'].get('markdown', '') for part in committed_journal_parts(events, database)
        if part['head']['metadata']['kind'] == 'text'
        and part['head']['metadata']['data'].get('is_final')
        and part['head']['metadata']['data'].get('visibility') == 'public')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--config', type=Path, default=Path.home()/'.nola/config.json')
    parser.add_argument('--binary', type=Path, default=ROOT/'target/debug/nola')
    parser.add_argument('--output', type=Path)
    parser.add_argument('--max-provider-requests', type=int, default=64)
    parser.add_argument('--collaboration-only', action='store_true', help='Run Main Agent to leaf expert acceptance (also the default)')
    args = parser.parse_args()
    assert 1 <= args.max_provider_requests <= 64, 'Use a bounded live request budget'
    upstream, model, secret = selected_route(args.config)
    output = (args.output or Path(tempfile.mkdtemp(prefix='nola-expert-role-real-'))).resolve()
    output.mkdir(parents=True, exist_ok=True); output.chmod(0o700)
    assert not (output/'store').exists(), 'Use a fresh isolated output directory'
    receipt = {'passed': False, 'cases': {}, 'provider_observations': [], 'request_limit': args.max_provider_requests, 'permissions': []}
    host = None

    def observe(request):
        if request.get('model') != model or not request.get('input'):
            receipt['locally_rejected_metadata_attempts'] = receipt.get('locally_rejected_metadata_attempts', 0)+1
            return
        if len(receipt['provider_observations']) >= receipt['request_limit']:
            raise RuntimeError('Live acceptance request limit reached')
        text = json.dumps(request, ensure_ascii=False)
        receipt['provider_observations'].append({
            'effort': (request.get('reasoning') or {}).get('effort'),
            'review_role': 'Prioritize actionable findings' in text,
            'code_role': 'Implement requested changes in the existing codebase' in text,
            'role_skill_reference': 'implement-code' in text,
            'tool_results_present': 'function_call_output' in text,
            'delegation_tools': sorted((item.get('name') or item.get('function', {}).get('name')) for item in request.get('tools', []) if (item.get('name') or item.get('function', {}).get('name')) in {'delegate_expert','expert_exchange','spawn_agent','request_user_input'}),
        })

    try:
        with observed_provider(upstream, model, secret, output/'provider.ndjson', request_observer=observe) as endpoint:
            project, env, config = fixture(output/'store', model, endpoint, profile='allow_all')
            binary = args.binary.resolve()
            # The public legacy profile editor intentionally cannot mutate
            # managed-connection profiles. Configure a new anonymous fixture
            # endpoint through its own public API instead of bypassing ownership.
            configuration = json.loads(config.read_text())
            configuration.pop('llm')
            # This is role acceptance, not the borrowed helper's forced 64K
            # compression stress case. Stay within the configured declaration.
            original = json.loads(args.config.read_text()).get('llm', {})
            declared_windows = [item.get('context_window') for connection in original.get('connections', [])
                for item in connection.get('models', []) if item.get('id') == model]
            declared = next((value for value in declared_windows if isinstance(value, int) and value >= 8192), 65536)
            window = min(declared, 131072)
            configuration['compaction'] = {'context_window_tokens': window,
                'token_threshold': window*3//4, 'keep_recent_messages': 8}
            receipt['fixture_context_window_tokens'] = window
            config.write_text(json.dumps(configuration))
            # Code workers require an owned checkout root. A fixture nested in
            # the source repository must never inherit that repository's worktree.
            git_env = {**env, 'GIT_CONFIG_GLOBAL': os.devnull, 'GIT_CONFIG_NOSYSTEM': '1', 'GIT_TERMINAL_PROMPT': '0'}
            subprocess.run(['git', 'init', '-q'], cwd=project, env=git_env, check=True, timeout=15)
            (project/'README.md').write_text('Owned, disposable expert-role acceptance workspace.\n')
            subprocess.run(['git', 'add', 'README.md'], cwd=project, env=git_env, check=True, timeout=15)
            subprocess.run(['git', '-c', 'user.name=Nola QA', '-c', 'user.email=qa@example.invalid',
                '-c', 'commit.gpgsign=false', 'commit', '-qm', 'test: initialize owned acceptance workspace'],
                cwd=project, env=git_env, check=True, timeout=15)
            actual_root = subprocess.check_output(['git', 'rev-parse', '--show-toplevel'], cwd=project, env=git_env, text=True).strip()
            assert Path(actual_root).resolve() == project.resolve(), 'Code tests must not use the production checkout'
            with socket.socket() as sock:
                sock.bind(('127.0.0.1', 0)); port = sock.getsockname()[1]
            base = f'http://127.0.0.1:{port}'
            with (output/'host.log').open('w') as log:
                host = subprocess.Popen([str(binary), 'web', '--host', '127.0.0.1', '--port', str(port),
                    '--web-root', str(ROOT/'apps/web'), '--project', str(project), '--allow-dir', str(project),
                    '--bash-prefix', 'python3', '--network-policy', 'deny', '--max-iterations', '64', '--debug'],
                    env=env, cwd=project, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
                opener = build_opener(HTTPCookieProcessor(http.cookiejar.CookieJar()))

                def api(path, body=None):
                    data = None if body is None else json.dumps(body).encode()
                    request = Request(base+path, data=data, headers={'Content-Type': 'application/json', 'Origin': base})
                    try:
                        with opener.open(request, timeout=30) as response:
                            return json.loads(response.read())
                    except HTTPError as error:
                        payload = json.loads(error.read())
                        raise RuntimeError(f'Isolated API {path}: {error.code}: {str(payload.get("code", ""))+": "+str(payload.get("error", "request failed"))}') from None

                def stream(path,body):
                    request=Request(base+path,data=json.dumps(body).encode(),headers={
                        'Content-Type':'application/json','Accept':'text/event-stream','Origin':base})
                    with opener.open(request,timeout=240) as response:
                        for line in response:
                            if not line.startswith(b'data:'):continue
                            data=line[5:].strip()
                            if data==b'[DONE]':break
                            if not data:continue
                            event=json.loads(data)
                            if event.get('type') in {'error','submission_rejected'}:
                                raise RuntimeError('Main source rejected: '+str(event.get('reason_code') or event.get('code') or 'unconfirmed'))
                            if event.get('type')=='submission_receipt' and event.get('receipt',{}).get('phase')=='rejected':
                                raise RuntimeError('Main source admission was rejected')

                deadline = time.monotonic()+45
                while time.monotonic() < deadline:
                    if host.poll() is not None: raise RuntimeError('Isolated Host exited during startup')
                    token = output/'store/home/.nola/web/mobile-access-token'
                    if token.exists():
                        try:
                            api('/api/web/login', {'token': token.read_text().strip()}); break
                        except (URLError, RuntimeError): pass
                    time.sleep(.2)
                else: raise TimeoutError('Isolated Host authentication')
                host_id = api('/api/web/session')['hostId']
                command = lambda value: api('/api/agent-rooms', value)
                catalog = command({'type': 'role_templates'})
                assert catalog['version'] == 1 and catalog['features']['explicit_generation_reasoning']
                roles = {role['role_id']: role for role in catalog['templates']}
                profiles = api('/api/model-profiles')['profiles']
                assert not profiles, 'The live acceptance Store must start without model profiles'
                profiles = api('/api/model-profiles', {'expectedRevisions': {}, 'endpointName': 'Owned acceptance relay',
                    'baseURL': endpoint, 'wireProtocol': 'responses', 'modelName': model, 'displayName': 'Owned live QA model',
                    'priority': 50, 'capabilities': ['chat'], 'executionCapabilities': {
                        'toolCalls': True, 'imageInput': False, 'reasoning': True, 'samplingParameters': False}})['profiles']
                assert len(profiles) == 1
                profile = profiles[0]
                # An explicit QA declaration, checked by the actual provider
                # below. It is not advertised as catalog-discovered capability.
                profiles = api(f'/api/model-profiles/{profile["id"]}/parameters', {
                    'expectedRevision': profile['revision'],
                    'parameters': {'maxOutputTokens': 8192},
                    'reasoning': {'control': 'effort', 'efforts': ['low', 'high'], 'defaultPreference': 'automatic'},
                })['profiles']
                profile = next(item for item in profiles if item['id'] == profile['id'])
                profiles = api(f'/api/model-profiles/{profile["id"]}/select', {'expectedRevision': profile['revision']})['profiles']
                profile = next(item for item in profiles if item['id'] == profile['id'])
                database = output/'store/nola.db'
                client = 'expert-role-acceptance-'+uuid.uuid4().hex

                from real_expert_collaboration import run_collaboration
                receipt['cases'].update(run_collaboration(api, stream, command, profile, roles, database,
                    host_id, client, output, receipt['provider_observations']))
                receipt['passed'] = True
    finally:
        if host is not None and host.poll() is None:
            os.killpg(host.pid, signal.SIGTERM)
            try: host.wait(timeout=10)
            except subprocess.TimeoutExpired: os.killpg(host.pid, signal.SIGKILL); host.wait(timeout=5)
        (output/'receipt.json').write_text(json.dumps(receipt, ensure_ascii=False, indent=2))
        print(json.dumps({'passed': receipt['passed'], 'cases': receipt['cases'], 'provider_requests': len(receipt['provider_observations']), 'output': str(output)}))


if __name__ == '__main__':
    main()
