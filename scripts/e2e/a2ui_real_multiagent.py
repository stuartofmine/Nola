#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Real room dispatch -> overlapping model Runs -> same-name A2UI surfaces.

Uses only product HTTP and the real Chrome flow. No Core injection or API mocks.
Requires an already-running isolated Host and an existing explicit model profile.
Does not launch models on import or --help. Failures retain isolated evidence.
"""
import argparse
import concurrent.futures
import http.cookiejar
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid


def assert_isolated(state):
    root = Path(state['root']).resolve(strict=True)
    url = urllib.parse.urlsplit(state['url'])
    if not root.name.startswith('nola-a2ui-real-') or url.scheme != 'http' or url.hostname != '127.0.0.1':
        raise ValueError('An explicitly isolated loopback A2UI Host is required')
    return root, urllib.parse.urlunsplit((url.scheme, url.netloc, '', '', ''))


def assert_surfaces(members, surfaces, markers):
    assert len(surfaces) == 2
    for member, surface, marker in zip(members, surfaces, markers):
        owner = surface['identity']['owner']
        assert owner['conversation_id'] == member['context_conversation_id']
        assert owner['context_id'] == member['context_conversation_id']
        assert owner['participant_id'] == member['id']
        assert surface['identity']['surface_id'] == 'shared-acceptance-form'
        assert surface['phase'] == 'ready'
        text = json.dumps(surface, ensure_ascii=False)
        assert marker in text and all(other not in text for other in markers if other != marker)
    assert surfaces[0]['identity'] != surfaces[1]['identity']


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--state', type=Path, required=True)
    parser.add_argument('--profile-id', required=True, help='Existing isolated Host model profile ID; never a credential')
    parser.add_argument('--timeout', type=int, default=300)
    parser.add_argument('--resume', type=Path, help='Resume observing an existing evidence directory; never resubmit its Runs')
    parser.add_argument('--browser-evidence', type=Path, help='Verify an already successful resumed browser attempt without submitting again')
    args = parser.parse_args()
    if args.browser_evidence and not args.resume:
        parser.error('--browser-evidence requires --resume')
    state = json.loads(args.state.read_text())
    root, base = assert_isolated(state)
    evidence = args.resume.resolve(strict=True) if args.resume else root / ('multiagent-' + str(uuid.uuid4()))
    if evidence.parent != root or not evidence.name.startswith('multiagent-'):
        raise ValueError('Evidence must belong to this isolated Host')
    evidence.mkdir(mode=0o700, exist_ok=args.resume is not None)
    def save(name, value):
        path = evidence / (name + '.json')
        path.write_text(json.dumps(value, ensure_ascii=False, indent=2)); path.chmod(0o600)
    def client():
        opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))
        def request(path, body=None):
            req = urllib.request.Request(base + path, data=None if body is None else json.dumps(body).encode(), headers={'Content-Type': 'application/json'})
            with opener.open(req, timeout=30) as response:
                return json.load(response)
        token = (root / 'home/.nola/web/mobile-access-token').read_text().strip()
        request('/api/web/login', {'token': token})
        return request
    request = client()
    profiles = request('/api/model-profiles')['profiles']
    profile = next((value for value in profiles if value['id'] == args.profile_id), None)
    if profile is None or not profile.get('enabled') or not profile['endpoint'].get('enabled'):
        raise ValueError('The explicit model profile and its endpoint must be enabled')
    if 'chat' not in profile.get('capabilities', []) or profile.get('executionCapabilityStatus') != 'tools_enabled':
        raise ValueError('The explicit model profile must support chat and tool calls')
    selection = {key: profile[key] for key in ('id', 'modelName', 'revision')}
    if args.resume:
        original_selection = json.loads((evidence / 'model-selection.json').read_text())
        if original_selection != selection:
            raise ValueError('Resume requires the original model profile and revision')
    else:
        save('model-selection', selection)
    room_command = lambda body: request('/api/agent-rooms', body)
    host = request('/api/web/session')['hostId']
    if args.resume:
        manifest = json.loads((evidence / 'manifest.json').read_text())
        agents, markers, members, submissions = (manifest[key] for key in ('agents', 'markers', 'members', 'submissions'))
        room = json.loads((evidence / 'room.json').read_text())
        room_id = room['room']['id']; source_client = 'room:' + room_id
    else:
        directory = room_command({'type': 'directory'})
        agents = ['a2ui-' + uuid.uuid4().hex[:12] for _ in range(2)]
        markers = ['MEMBER_' + uuid.uuid4().hex for _ in range(2)]
        for agent in agents:
            directory = room_command({'type': 'configure_agent', 'input': {
                'id': agent, 'name': agent, 'description': 'Generates interactive forms for this participant.',
                'expected_directory_revision': directory['revision'], 'expected_agent_revision': None,
                'defaults': {'modelSelectionID': 'configured:' + args.profile_id, 'modelOverrides': {}, 'memoryEnabledOverride': False},
                'availability': 'active', 'updated_at_ms': int(time.time()*1000)}})
        room = room_command({'type': 'create', 'title': 'A2UI concurrent participants', 'at_ms': int(time.time()*1000)})
        for agent in agents:
            room = room_command({'type': 'join', 'input': {'room_id': room['room']['id'], 'expected_room_revision': room['revision'],
                'agent_id': agent, 'expected_agent_revision': 1, 'at_ms': int(time.time()*1000)}})
        members = [member for member in room['room']['participants'] if member['agent_id'] in agents and member['active']]
        assert len(members) == 2 and len(room['room']['participants']) == 2, 'Fresh room must contain exactly the two newly joined agents'
        members.sort(key=lambda member: agents.index(member['agent_id']))
        room_id = room['room']['id']; source_client = 'room:' + room_id
        save('room', room)
        submissions = [str(uuid.uuid4()) for _ in members]
        def publish(index):
            send = client()
            return send('/api/agent-rooms', {'type': 'submit', 'input': {'room_id': room_id,
                'submission_id': submissions[index], 'expected_room_revision': room['revision'],
                'participant_id': members[index]['id'], 'at_ms': int(time.time()*1000),
                'text': f'请实际使用 render_ui 创建 A2UI 0.9.1 表单，surfaceId 为 shared-acceptance-form。一个名称输入框默认值 {markers[index]}，一个 event action 提交按钮。提交后的值由你处理并更新原表单显示。请遵守工具提供的 schema，不读写文件，不用 Markdown 代替界面。'}})
        save('manifest', {'agents': agents, 'markers': markers, 'members': members, 'submissions': submissions})
        with concurrent.futures.ThreadPoolExecutor(max_workers=2) as executor:
            accepted = list(executor.map(publish, range(2)))
        save('accepted', accepted)
    observations = json.loads((evidence / 'observations.json').read_text()) if args.resume and (evidence / 'observations.json').exists() else []
    overlap = any(all(value and value['status'] == 'running' and value['worker']['state'] == 'running' for value in sample['runs']) for sample in observations)
    approvals = set(); deadline = time.monotonic() + args.timeout
    while time.monotonic() < deadline:
        rows = room_command({'type': 'submissions', 'room_id': room_id, 'limit': 100, 'offset': 0})
        selected = [next(row for row in rows if row['input']['submission_id'] == sid) for sid in submissions]
        snapshots = []
        for member, row in zip(members, selected):
            if not row.get('execution_id'):
                snapshots.append(None); continue
            try:
                observed = request('/api/chat/agent/v2/observe', {'operation': 'observe', 'expected_host_id': host,
                    'client_id': source_client, 'conversation_id': member['context_conversation_id'],
                    'expected_run_id': row['execution_id'], 'assistant_message_id': row['execution_id'] + ':assistant'})['result']
            except urllib.error.HTTPError as error:
                if error.code != 409:
                    raise
                # The room claim can become visible before BeginTurn commits.
                # Retry only observation of this exact Run, never submission.
                snapshots.append(None)
                continue
            snapshots.append(observed)
            for permission in observed['permissions']:
                if permission['nonce'] in approvals: continue
                assert permission['canonical_run_id'] == row['execution_id'] and permission['client_id'] == source_client
                decision = {key: permission[key] for key in ('protocol_version', 'nonce', 'request_id', 'host_id', 'client_id', 'session_id', 'tool_name')}
                decision['resolution'] = {'type': 'authorization', 'decision': 'allow_once' if permission['tool_name'] == 'render_ui' else 'deny'}
                request('/api/agent/host-permission/decision', decision); approvals.add(permission['nonce'])
        overlap |= all(value and value['status'] == 'running' and value['worker']['state'] == 'running' for value in snapshots)
        observations.append({'at_ms': int(time.time()*1000), 'runs': snapshots})
        save('observations', observations)
        if all(row['state'] == 'completed' for row in selected): break
        assert not any(row['state'] in ('failed', 'blocked', 'incomplete', 'cancelled') for row in selected), 'Room Run did not succeed'
        time.sleep(0.5)
    else: raise AssertionError('Timed out waiting for both real room Runs')
    assert overlap, 'No overlapping executing Runs observed; sequential completion is not concurrency evidence'
    def surfaces(member):
        return request('/api/chat/conversations/' + member['context_conversation_id'] + '/ui-surfaces')['surfaces']
    before = json.loads((evidence / 'surfaces-before.json').read_text()) if args.browser_evidence else [next(surface for surface in surfaces(member) if surface['identity']['surface_id'] == 'shared-acceptance-form') for member in members]
    assert_surfaces(members, before, markers); save('surfaces-before', before)
    # Existing real browser runner edits/submits, verifies actual model terminal,
    # typed action summary, natural original-Surface update and mobile reload.
    repo = Path(__file__).resolve().parents[2]
    if args.browser_evidence:
        browser_path = args.browser_evidence.resolve(strict=True)
        recorded = json.loads((evidence / 'chrome-evidence.json').read_text())
        if browser_path != Path(recorded['path']).resolve(strict=True):
            raise ValueError('Browser evidence must be the original recorded attempt')
        result = json.loads((browser_path / 'result.json').read_text())
        required = {'submit', 'action-terminal-with-value', 'same-client-reload', 'natural-followup-terminal', 'same-surface-update', 'mobile-render'}
        assert result['conversation'] == members[0]['context_conversation_id']
        assert not result.get('failed') and result.get('errors') == [] and required <= set(result.get('passed', []))
    else:
        chrome_evidence = Path(tempfile.mkdtemp(prefix='nola-a2ui-multiagent-'))
        chrome_log = evidence / (chrome_evidence.name + '.log')
        save('chrome-evidence', {'path': str(chrome_evidence), 'client_id': source_client, 'log': str(chrome_log)})
        env = dict(os.environ, NOLA_A2UI_TEST_CONVERSATION=members[0]['context_conversation_id'],
            NOLA_A2UI_EVIDENCE_DIR=str(chrome_evidence), NOLA_A2UI_CLIENT_ID=source_client)
        for name in ('NOLA_A2UI_RESUME', 'NOLA_A2UI_RESUME_FOLLOWUP', 'NOLA_A2UI_ISOLATION_CONVERSATION'):
            env.pop(name, None)
        env['NOLA_A2UI_HOST_STATE'] = str(args.state.resolve(strict=True))
        result = subprocess.run(['node', str(repo / 'apps/web/playwright_runtime/a2ui_real_flow.cjs')], env=env,
            stdout=chrome_log.open('w'), stderr=subprocess.STDOUT, timeout=args.timeout*2)
        assert result.returncode == 0, 'Real Chrome action/follow-up failed; inspect chrome-evidence.json for the attempt log'
    after = [next(surface for surface in surfaces(member) if surface['identity'] == original['identity']) for member, original in zip(members, before)]
    assert after[0]['revision'] > before[0]['revision']; assert after[1] == before[1], 'Member B changed after A action'
    save('surfaces-after', after)
    room = room_command({'type': 'read', 'room_id': room_id})
    room_command({'type': 'deactivate', 'room_id': room_id, 'participant_id': members[0]['id'], 'expected_revision': room['revision'], 'at_ms': int(time.time()*1000)})
    button = next(node for node in after[0]['nodes'].values() if node.get('actions'))
    action = next(iter(button['actions'].values()))
    body = {'submission_id': str(uuid.uuid4()), 'submitted_at_ms': int(time.time()*1000), 'interaction': {
        'identity': after[0]['identity'], 'expected_surface_revision': after[0]['revision'], 'edits': [], 'action': action}}
    rejection = request('/api/chat/conversations/' + members[0]['context_conversation_id'] + '/ui-interactions',
        {'request_id': body['submission_id'], 'input': body})
    save('removed-member-rejection', rejection)
    assert rejection['status'] == 'rejected', 'Inactive participant must be explicitly rejected, not unknown/conflict or malformed HTTP'
    assert surfaces(members[1]) == [after[1]], 'Removing A must not alter B Surface'
    save('result', {'passed': True, 'overlapping_runs': True, 'same_wire_surface_id': True,
        'member_action_model_terminal': True, 'other_member_unchanged': True, 'removed_member_rejected': True})
    print('PASS real multi-Agent A2UI; evidence:', evidence)

if __name__ == '__main__':
    main()
