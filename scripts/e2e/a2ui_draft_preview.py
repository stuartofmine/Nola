#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Read-only draft preview against an existing isolated real Host Surface.

Does not publish, submit an action, change configuration, or invoke a model.
Core required-field semantics are covered by the separate generated fixture.
"""
import argparse
import copy
import http.cookiejar
import json
from pathlib import Path
import tempfile
import urllib.error
import urllib.request
from urllib.parse import quote, urlsplit


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--state', required=True, type=Path)
    parser.add_argument('--conversation', required=True)
    args = parser.parse_args()
    state = json.loads(args.state.read_text())
    root = Path(state['root']).resolve(strict=True)
    address = urlsplit(state['url'])
    assert root.name.startswith('nola-a2ui-real-')
    assert address.scheme == 'http' and address.hostname == '127.0.0.1'
    opener = urllib.request.build_opener(
        urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))

    def request(path, body=None, expected=200):
        req = urllib.request.Request(state['url'] + path,
            data=None if body is None else json.dumps(body).encode(),
            headers={'Content-Type': 'application/json'})
        try:
            response = opener.open(req, timeout=30)
        except urllib.error.HTTPError as error:
            response = error
        with response:
            assert response.code == expected, f'Unexpected HTTP {response.code} for {path}'
            return json.load(response)

    token = (root / 'home/.nola/web/mobile-access-token').read_text().strip()
    request('/api/web/login', {'token': token})
    route = '/api/chat/conversations/' + quote(args.conversation, safe='')
    before = request(route + '/ui-surfaces')
    surface = next(s for s in before['surfaces'] if s['phase'] == 'ready'
        and any(n['component'] == 'TextField' and 'value' in n['bindings']
            and not n['props'].get('disabled', False) for n in s['nodes'].values()))
    field = next(n for n in surface['nodes'].values() if n['component'] == 'TextField'
        and 'value' in n['bindings'] and not n['props'].get('disabled', False))
    marker = '只读草稿 Preview — 未提交'
    body = {'identity': surface['identity'], 'expected_surface_revision': surface['revision'],
        'draft_generation': 17, 'edits': [{'type': 'set', 'instance_id': field['instance_id'],
            'binding': 'value', 'value': marker}]}
    result = request(route + '/ui-surface-preview', body)['preview']
    assert result['identity'] == surface['identity']
    assert result['base_revision'] == result['snapshot']['revision'] == surface['revision']
    assert result['draft_generation'] == 17
    assert result['snapshot']['nodes'][field['instance_id']]['props']['value'] == marker
    assert request(route + '/ui-surfaces') == before, 'Preview changed committed projection'
    invalid = copy.deepcopy(body)
    invalid['expected_surface_revision'] += 1
    request(route + '/ui-surface-preview', invalid, 409)
    invalid = copy.deepcopy(body)
    invalid['identity']['owner']['conversation_id'] = 'different-conversation'
    request(route + '/ui-surface-preview', invalid, 400)
    invalid = copy.deepcopy(body)
    invalid['action'] = {'name': 'submit'}
    request(route + '/ui-surface-preview', invalid, 400)
    assert request(route + '/ui-surfaces') == before
    evidence = Path(tempfile.mkdtemp(prefix='nola-a2ui-preview-http-'))
    report = {'passed': True, 'conversation': args.conversation,
        'identity': surface['identity'], 'baseRevision': surface['revision'],
        'candidateValue': True, 'committedProjectionUnchanged': True,
        'staleRejected': True, 'wrongConversationRejected': True,
        'actionRejected': True, 'modelRequested': False}
    (evidence / 'result.json').write_text(json.dumps(report, ensure_ascii=False, indent=2))
    print(json.dumps({'passed': True, 'evidence': str(evidence)}))


if __name__ == '__main__':
    main()
