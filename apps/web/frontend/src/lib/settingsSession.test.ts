import { test } from 'node:test';
import assert from 'node:assert/strict';
import { settingsSession, logoutConfirmed } from './settingsSession.ts';

test('settings distinguish browser authentication from local host trust', () => {
    const local = settingsSession({ authenticated: true, sessionAuthenticated: false, localTrust: true, hostId: 'host', transportIngress: 'direct', token: 'never-retain' });
    assert.equal(local.sessionAuthenticated, false);
    assert.equal('token' in local, false);
    assert.equal(settingsSession({ authenticated: true, sessionAuthenticated: true, localTrust: false, hostId: 'remote', transportIngress: 'tailcat' }).localTrust, false);
    assert.equal(settingsSession({ authenticated: false, sessionAuthenticated: false, localTrust: false, hostId: 'remote', transportIngress: 'tailcat' }).authenticated, false);
    for (const value of [null, [], {}, { ...local, authenticated: false }, { ...local, hostId: '' }, { ...local, sessionAuthenticated: 'true' }, { ...local, transportIngress: 'tailcat' }, { ...local, transportIngress: 'unknown' }]) {
        assert.throws(() => settingsSession(value));
    }
});

test('logout requires an explicit negative authentication receipt, not any successful JSON', () => {
    for (const value of [null, {}, [], { ok: true }, { authenticated: true }, { authenticated: 'false' }]) assert.equal(logoutConfirmed(value), false);
    assert.equal(logoutConfirmed({ authenticated: false }), true);
});
