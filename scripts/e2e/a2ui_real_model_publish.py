#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Publish a form through an explicitly isolated real Web Host; no API mocks.

The state file must contain root and a loopback url. The Host must already have
an imported real model. Credentials are read only into memory; evidence is kept
under the isolated root. Only render_ui approval is granted by this scenario.
"""
import argparse
import pathlib
import json
import urllib.request
import urllib.error
import http.cookiejar
import uuid
from collections import Counter
from urllib.parse import urlparse
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--state', required=True, type=pathlib.Path)
args = parser.parse_args()
s = json.loads(args.state.read_text())
root = pathlib.Path(s['root'])
base = s['url']
parsed = urlparse(base)
if parsed.scheme != 'http' or parsed.hostname != '127.0.0.1' or (not root.name.startswith('nola-a2ui-real-')):
    raise SystemExit('Only an explicitly isolated loopback A2UI Host is accepted')
root = root.resolve(strict=True)
evidence = root / ('publish-' + str(uuid.uuid4()))
evidence.mkdir(mode=0o700)
token = (root / 'home/.nola/web/mobile-access-token').read_text().strip()
opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))

def request(path, body=None):
    r = urllib.request.Request(base + path, data=None if body is None else json.dumps(body).encode(), headers={'Content-Type': 'application/json'})
    with opener.open(r, timeout=60) as f:
        return json.load(f)
request('/api/web/login', {'token': token})
request('/api/agent/settings', {'agent_mode_enabled': True, 'host_tools_enabled': True, 'enable_local_dev_tools': False, 'network_policy': 'allow', 'allowed_dirs': [str(root)]})
c = request('/api/chat/conversations', {'title': 'A2UI real form acceptance', 'selectedModelSource': 'auto', 'selectedModelId': ''})
cid = c['conversationId']
assert cid, c
(evidence / 'conversation.json').write_text(json.dumps(c, ensure_ascii=False, indent=2))
host = request('/api/web/session')['hostId']
sid = str(uuid.uuid4())
body = {'conversationId': cid, 'submission_id': sid, 'user_message_id': sid + ':user', 'assistant_message_id': sid + ':assistant', 'client_id': 'a2ui-real-acceptance', 'messages': [{'role': 'user', 'content': '请使用 render_ui 为我创建一个原生表单，填写这次测试的名称，默认名称是 A2UI 验收；有一个提交按钮。请直接生成可编辑界面，不要用 Markdown 或代码块代替，也不要读写文件。'}], 'language': 'zh'}
body.update(expected_host_id=host, expected_configuration_revision=c['configurationRevision'], expected_parent_message_id=None, execution_backend=c['executionBackend'])
plan = request('/api/agent/tool-plan/prepare', {'expected_host_id': host, 'client_id': body['client_id'], 'session_id': cid})
body['tool_plan_ref'] = plan['result']['reference']
identity = {'source_host_id': host, 'client_id': body['client_id'], 'conversation_id': cid, 'submission_id': sid, 'user_message_id': body['user_message_id'], 'assistant_message_id': body['assistant_message_id']}
prepared = request('/api/chat/agent/v2/submission', {'operation': 'prepare', 'identity': identity, 'request': body})
body['request_digest'] = prepared['result']['request_digest']
(evidence / 'request.json').write_text(json.dumps(body, ensure_ascii=False, indent=2))
r = urllib.request.Request(base + '/api/chat/agent/v2/stream', data=json.dumps(body).encode(), headers={'Content-Type': 'application/json', 'Accept': 'text/event-stream'})
event_counts = Counter()
terminal_traces = []
try:
    with opener.open(r, timeout=180) as f, (evidence / 'stream.ndjson').open('w') as log:
        for line in f:
            text = line.decode()
            log.write(text)
            log.flush()
            if text.startswith('data:'):
                payload = text[5:].strip()
                if payload and payload != '[DONE]':
                    event = json.loads(payload)
                    event_counts[event.get('type', 'unknown')] += 1
                    trace = event.get('trace', {})
                    if trace.get('kind') in ('run.completed', 'run.failed', 'run.cancelled'):
                        terminal_traces.append(trace)
                    if event.get('type') == 'permission_request':
                        tool = event['tool_name']
                        decision = {k: event.get(k, '') for k in ['nonce', 'host_id', 'client_id', 'session_id']}
                        decision.update(protocol_version=event.get('protocol_version', 0), request_id=event['request_id'], tool_name=tool, resolution={'type': 'authorization', 'decision': 'allow_once' if tool == 'render_ui' else 'deny'})
                        request('/api/agent/host-permission/decision', decision)
    surfaces = request('/api/chat/conversations/' + cid + '/ui-surfaces')
    (evidence / 'surfaces.json').write_text(json.dumps(surfaces, ensure_ascii=False, indent=2))
    assert any(value['phase'] == 'ready' for value in surfaces['surfaces']), 'Real model produced no ready Surface; inspect isolated evidence'
    messages = request('/api/chat/conversations/' + cid + '/messages')
    (evidence / 'messages.json').write_text(json.dumps(messages, ensure_ascii=False, indent=2))
    assert messages['conversationProjection']['active_run'] is None, 'Run is not terminal; publication alone is not completion'
    report = {'event_counts': dict(event_counts), 'terminal_traces': terminal_traces,
              'surface_revisions': [value['revision'] for value in surfaces['surfaces']]}
    (evidence / 'verification.json').write_text(json.dumps(report, ensure_ascii=False, indent=2))
    assert any(trace.get('kind') == 'run.completed' and trace.get('status') == 'succeeded'
               and trace.get('run_id') == sid for trace in terminal_traces), 'Surface exists but the submitted Run did not report successful completion'
    print(json.dumps({'conversation_id': cid, 'surface_count': len(surfaces.get('surfaces', [])), 'evidence': str(evidence)}, ensure_ascii=False))
except urllib.error.HTTPError as e:
    (evidence / 'http-error.txt').write_bytes(e.read())
    print('HTTP error:', e.code, 'evidence:', evidence)
    raise SystemExit(1)
