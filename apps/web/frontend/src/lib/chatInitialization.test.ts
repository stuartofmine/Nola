import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { ApiError } from './api.ts';
import { chatInitializationError, parseInitializationSession } from './chatInitialization.ts';

test('initialization separates authentication, service/network and response contract failures without leaking stderr', () => {
    for (const status of [401, 403]) assert.equal(chatInitializationError(new ApiError(status, 'private stderr')), 'chatPage.noSession');
    for (const status of [404, 409, 500, 503]) assert.equal(chatInitializationError(new ApiError(status, '/private/database schema mismatch')), 'chatPage.initializationService');
    assert.equal(chatInitializationError(new TypeError('network failed')), 'chatPage.initializationService');
    assert.equal(chatInitializationError(new Error('chatConversations.invalidResponse')), 'chatPage.initializationContract');
    assert.equal(chatInitializationError(new SyntaxError('private body')), 'chatPage.initializationContract');
    assert.equal(chatInitializationError(new Error('notification_host_changed')), 'chatPage.initializationHostChanged');
});

test('session contract distinguishes explicit unauthenticated from malformed successful responses', () => {
    assert.deepEqual(parseInitializationSession({authenticated: true, hostId: 'host'}), {hostId: 'host'});
    for (const value of [null, {}, {authenticated: true}, {authenticated: true, hostId: 42}]) {
        try { parseInitializationSession(value); assert.fail('must reject malformed response'); }
        catch (error) { assert.equal(chatInitializationError(error), 'chatPage.initializationContract'); }
    }
    assert.throws(() => parseInitializationSession({authenticated: false}), /invalid_host_session/);
});

test('retry is bound to initialization reads, not submit or resume', () => {
    const source = readFileSync(new URL('../pages/Chat.svelte', import.meta.url), 'utf8');
    const initialize = source.slice(source.indexOf('async function initialize()'), source.indexOf('onMount(() =>'));
    assert.match(initialize, /initializationInFlight/);
    assert.doesNotMatch(initialize, /\.submit\(|\.resume\(|method\s*:/);
    assert.match(source, /onclick=\{\(\) => void initialize\(\)\} disabled=\{initializing\}/);
});

test('missing Web Locks explains the secure connection requirement separately from storage failure', () => {
    assert.equal(chatInitializationError(new Error('browser_secure_connection_required')), 'chatPage.secureConnectionRequired');
    assert.equal(chatInitializationError(new Error('browser_recovery_storage_unavailable')), 'chatPage.recoveryStorageUnavailable');
});
