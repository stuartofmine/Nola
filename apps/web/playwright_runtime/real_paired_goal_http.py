"""Encrypted production HTTP checks after a real browser-created paused Goal.

No model or response mocks. Uses only the explicitly supplied isolated Host.
"""
import base64
import hashlib
import json
import os
import pathlib
import urllib.error
import urllib.request
from cryptography.hazmat.primitives.ciphers.aead import AESGCM

state = json.loads(pathlib.Path(os.environ['NOLA_BUTTON_AUDIT_HOST_STATE']).read_text())
root = pathlib.Path(state['root'])
assert root.name.startswith('nola-a2ui-real-')
prepared = json.loads(pathlib.Path(os.environ['NOLA_GOAL_PREPARATION_REPORT']).read_text())
assert prepared['passed'] and prepared['pausedForNativeObservation']
conversation = prepared['conversationId']
token = (root / 'home/.nola/web/mobile-access-token').read_text().strip()
host = (root / 'home/.nola/web/host-identity').read_text().strip()
cipher = AESGCM(hashlib.sha256(token.encode()).digest())
report = {'conversation_id': conversation, 'requests': [], 'passed': False}

def encode(value):
    return base64.urlsafe_b64encode(value).decode().rstrip('=')

def decode(value):
    return base64.urlsafe_b64decode(value + '=' * (-len(value) % 4))

def request(endpoint, value, authorized=True):
    nonce = os.urandom(12)
    sealed = cipher.encrypt(nonce, json.dumps(value).encode(), b'nola-mobile-bridge-v1')
    body = json.dumps({'type': 'encrypted', 'v': 1, 'alg': 'AES-256-GCM',
        'nonce': encode(nonce), 'ciphertext': encode(sealed[:-16]), 'tag': encode(sealed[-16:])}).encode()
    headers = {'Content-Type': 'application/json'}
    if authorized:
        headers['Authorization'] = 'Bearer ' + token
    req = urllib.request.Request(state['url'] + '/api/mobile/chat/agent/v2/' + endpoint,
        data=body, headers=headers, method='POST')
    try:
        response = urllib.request.urlopen(req, timeout=30)
    except urllib.error.HTTPError as error:
        response = error
    status = response.status
    envelope = json.load(response)
    encrypted = envelope.get('type') == 'encrypted'
    if encrypted:
        assert envelope['v'] == 1 and envelope['alg'] == 'AES-256-GCM'
        envelope = json.loads(cipher.decrypt(decode(envelope['nonce']),
            decode(envelope['ciphertext']) + decode(envelope['tag']), b'nola-mobile-bridge-v1'))
    report['requests'].append({'endpoint': endpoint, 'status': status, 'encrypted': encrypted,
        'operation_id': envelope.get('operationId'), 'authoritative': envelope.get('authoritative')})
    if authorized:
        assert encrypted, 'paired response must use authenticated encryption'
    return status, envelope

scope = {'expected_host_id': host, 'conversation_id': conversation}
try:
    assert request('goal', {**scope, 'control': {'action': 'cancel'}}, False)[0] == 401
    status, before = request('progress', scope)
    assert status == 200 and before['source_host_id'] == host and before['conversation_id'] == conversation
    goal = before['goal_projection']
    assert goal['active']['status'] == 'paused'
    control = {'operationId': 'paired-http-cancel', 'action': 'cancel', 'expectedGoalId': goal['active']['id'], 'expectedControlToken': goal['control_token']}
    status, foreign = request('goal', {**scope, 'expected_host_id': 'foreign-host', 'control': control})
    assert status == 409 and foreign['code'] == 'goal_control_source_mismatch'
    assert request('progress', scope)[1]['goal_projection'] == goal
    start_token = next(value['observedControlToken'] for value in prepared['receipts'] if value.get('status') == 'running')
    status, stale = request('goal', {**scope, 'control': {**control, 'operationId': 'paired-http-stale', 'expectedControlToken': start_token}})
    assert status == 409 and stale['source_host_id'] == host and stale['authoritative'] is True
    assert stale['goalProjection']['active']['status'] == 'paused'
    assert stale['goalProjection']['applied_operations'][stale['operationId']]['rejection']
    status, cancelled = request('goal', {**scope, 'control': control})
    assert status == 200 and cancelled['authoritative'] is True and cancelled['source_host_id'] == host
    assert cancelled['goalProjection']['active']['status'] == 'cancelled'
    assert cancelled['goalProjection']['applied_operations'][cancelled['operationId']]['rejection'] is None
    status, repeated = request('goal', {**scope, 'control': control})
    assert status == 200 and repeated == cancelled, 'same intent must return the original committed outcome'
    status, conflict = request('goal', {**scope, 'control': {**control, 'action': 'resume'}})
    assert status == 409, 'an operation identity cannot be reused for another action'
    final = request('progress', scope)[1]
    assert final['goal_projection'] == cancelled['goalProjection']
    report['goal_id'] = goal['active']['id']
    report['final_status'] = 'cancelled'
    report['passed'] = True
finally:
    output = pathlib.Path(os.environ.get('NOLA_PAIRED_GOAL_HTTP_REPORT', str(pathlib.Path(__file__).resolve().parents[3] / '.artifacts/nola-paired-goal-http-report.json')))
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(report, indent=2))
    print(json.dumps({'passed': report['passed'], 'requests': len(report['requests']), 'report': str(output)}))
