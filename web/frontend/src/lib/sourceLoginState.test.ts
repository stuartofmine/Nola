import test from 'node:test';
import assert from 'node:assert/strict';
import {sourceLoginSnapshot} from './sourceLoginState.ts';
const handle = {request_id: 'request', account_id: 'account', expected_revision: 3};
const pending = {request_id: 'request', state: 'pending', authorization_url: 'https://auth.openai.com/authorize',
    inference_supported: false, account: null};
test('source login preserves exact operation and never grants inference', () => {
    assert.equal(sourceLoginSnapshot(pending, handle).state, 'pending');
    for (const change of [{request_id: 'old'}, {inference_supported: true}, {state: 'ready'}])
        assert.throws(() => sourceLoginSnapshot({...pending, ...change}, handle));
});
test('source completion requires the exact newer account receipt', () => {
    const complete = {...pending, state: 'authenticated_source_only', authorization_url: null,
        account: {revision: 4, account: {id: 'account'}, secret: 'never project'}};
    assert.deepEqual(sourceLoginSnapshot(complete, handle).account, {revision: 4, account: {id: 'account'}});
    for (const account of [null, {revision: 3, account: {id: 'account'}}, {revision: 4, account: {id: 'other'}}])
        assert.throws(() => sourceLoginSnapshot({...complete, account}, handle));
});
test('authorization navigation accepts only the intended HTTPS login origins', () => {
    for (const authorization_url of ['javascript:alert(1)', 'https://attacker.example/',
        'https://auth.openai.com.attacker.example/', 'https://user@auth.openai.com/', 'http://chatgpt.com/'])
        assert.throws(() => sourceLoginSnapshot({...pending, authorization_url}, handle));
    assert.throws(() => sourceLoginSnapshot({...pending, state: 'cancelled'}, handle));
});
