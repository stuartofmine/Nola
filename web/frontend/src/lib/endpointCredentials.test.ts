import { test } from 'node:test';
import assert from 'node:assert/strict';
import { credentialSnapshot, credentialRequest, credentialPath, isValidCredentialToken } from './endpointCredentials.ts';

const status = () => ({ endpointID: 'provider/中文', baseURL: 'https://example.com/v1', wireProtocol: 'responses',
    expectedRevisions: { model: 3 }, credentialRevision: 2, hasCredential: true });

test('credential tokens match the bounded ASCII bearer contract before submission', () => {
    for (const token of ['', '   ', 'a b', 'a\nb', 'a\tb', 'a\0b', 'clé', '密钥', 'a\u007fb', 'a'.repeat(8193)]) {
        assert.equal(isValidCredentialToken(token), false);
        assert.throws(() => credentialRequest(status(), 'replace', token), /credential.invalidToken/);
    }
    for (const token of ['!', '~', 'sk-fixture_123.+/=:', 'a'.repeat(8192), '  key-fixture  ']) {
        assert.equal(isValidCredentialToken(token), true);
        assert.equal(credentialRequest(status(), 'replace', token).secret, token.trim());
    }
});

test('credential snapshots accept only public fields and freeze both revision baselines', () => {
    const source = { ...status(), secret: 'must-not-retain', unrelated: { secret: 'hidden' } };
    const snapshot = credentialSnapshot(source, source.endpointID);
    source.expectedRevisions.model = 99;
    assert.equal(snapshot.expectedRevisions.model, 3);
    assert.equal('secret' in snapshot, false);
    assert.equal('unrelated' in snapshot, false);
    assert.equal(credentialPath(snapshot.endpointID), '/api/model-endpoints/provider%2F%E4%B8%AD%E6%96%87/credential');
});

test('credential replacement and explicit clearing cannot be confused', () => {
    const snapshot = status();
    assert.deepEqual(credentialRequest(snapshot, 'replace', 'new-key'), {
        expectedRevisions: { model: 3 }, baseURL: snapshot.baseURL, wireProtocol: 'responses',
        credentialRevision: 2, action: 'replace', secret: 'new-key',
    });
    const cleared = credentialRequest(snapshot, 'clear');
    assert.equal('secret' in cleared, false);
    assert.equal('hasCredential' in cleared, false);
    assert.throws(() => credentialRequest(snapshot, 'clear', 'new-key'));
    assert.throws(() => credentialRequest(snapshot, 'replace', '   '));
});

test('credential operations reject missing identity, membership and invalid revisions', () => {
    assert.throws(() => credentialSnapshot(status(), 'other'));
    assert.throws(() => credentialPath(''));
    for (const credentialRevision of [-1, NaN, Infinity, 1.5, Number.MAX_SAFE_INTEGER + 1]) {
        assert.throws(() => credentialSnapshot({ ...status(), credentialRevision }, status().endpointID));
    }
    assert.throws(() => credentialSnapshot({ ...status(), expectedRevisions: {} }, status().endpointID));
    assert.throws(() => credentialSnapshot({ ...status(), expectedRevisions: { model: -1 } }, status().endpointID));
});

test('opaque member IDs do not alter credential request object prototypes', () => {
    const source = { ...status(), expectedRevisions: JSON.parse('{"__proto__":4,"constructor":5}') };
    const payload = credentialRequest(source, 'clear');
    assert.deepEqual(Object.keys(payload.expectedRevisions), ['__proto__', 'constructor']);
    assert.equal(payload.expectedRevisions.__proto__, 4);
});
