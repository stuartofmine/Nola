import { test } from 'node:test';
import assert from 'node:assert/strict';
import { safeLoginNext, validHostToken, loginLanguage, loginErrorKind } from './login.ts';

test('login destinations remain on the host and exclude normalized authentication/API loops', () => {
    for (const value of [null, '', 'https://evil.invalid', '//evil.invalid', '/\\evil.invalid', '/x/..//evil.invalid', '/chat\n', '/login', '/api', '/api/models', '/chat/../api/models', '/x/%2e%2e/login']) {
        assert.equal(safeLoginNext(value), '/dashboard', String(value));
    }
    assert.equal(safeLoginNext('/chat?conversation=opaque%2Fid#latest'), '/chat?conversation=opaque%2Fid#latest');
    assert.equal(safeLoginNext('/models'), '/models');
    assert.equal(safeLoginNext('/x/../models'), '/models');
});

test('host tokens are validated without accepting overlong or short values', () => {
    assert.equal(validHostToken('x'.repeat(31)), false);
    assert.equal(validHostToken('x'.repeat(32)), true);
    assert.equal(validHostToken(`  ${'x'.repeat(256)}  `), true);
    assert.equal(validHostToken('x'.repeat(257)), false);
});

test('login language and errors have deterministic non-secret presentation', () => {
    assert.equal(loginLanguage(['zh-CN', 'en']), 'zh');
    assert.equal(loginLanguage(['en-US', 'zh']), 'en');
    assert.equal(loginLanguage(['de']), 'en');
    assert.equal(loginErrorKind('too_many_login_attempts'), 'limited');
    assert.equal(loginErrorKind('invalid_host_token'), 'invalid');
    assert.equal(loginErrorKind('expired_handoff'), 'handoff');
    assert.equal(loginErrorKind('arbitrary server detail'), 'network');
});
