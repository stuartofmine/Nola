import test from 'node:test';
import assert from 'node:assert/strict';
import { kimiCanStartLogin } from './kimiLoginState.ts';

test('Kimi requires an observed terminal auth state before starting a source login', () => {
    for (const state of ['unknown', 'unconfirmed', 'pending', 'starting', '', 'future-state'])
        assert.equal(kimiCanStartLogin(state), false);
    for (const state of ['authenticated', 'unauthenticated', 'expired', 'revoked', 'denied', 'cancelled'])
        assert.equal(kimiCanStartLogin(state), true);
});
