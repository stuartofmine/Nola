import test from 'node:test';
import assert from 'node:assert/strict';
import { createSourceSubmission, bindSourceDigest, acceptSourceReceipt, sourceSubmissionBody, sourceSubmissionQuery,
    exactSubmissionCancel, SourceSubmissionStorage, parseObservedSourceReceipt } from './sourceSubmission.ts';

const identity = { source_host_id: 'host', client_id: 'client', conversation_id: 'conversation', submission_id: 'submission',
    user_message_id: 'user', assistant_message_id: 'assistant' };
const request = { conversationId: 'conversation', expected_host_id: 'host', client_id: 'client', submission_id: 'submission',
    user_message_id: 'user', assistant_message_id: 'assistant', messages: [{ role: 'user', content: '完整原始请求🙂' }], language: 'zh-Hans', timezone: 'Asia/Shanghai' };
const digest = 'a'.repeat(64);
const bound = () => bindSourceDigest(createSourceSubmission(identity, request), { source_host_id: 'host', result: { identity, request_digest: digest } });
const admission = { identity: { source_store_epoch: 'epoch', conversation_id: 'conversation', run_id: 'submission', assistant_message_id: 'assistant' }, sequence: '2', record_digest: 'b'.repeat(64) };
const receipt = (phase: 'prepared' | 'admitted' | 'rejected' | 'cancelled') => ({ protocol_version: 1, identity, request_digest: digest,
    phase, admission: phase === 'admitted' ? admission : null, rejection_code: phase === 'rejected' ? 'invalid_input' : null });

test('source-only digest preparation never admits a Run and original request remains exact', () => {
    const original = structuredClone(request); const pending = createSourceSubmission(identity, original);
    original.messages[0].content = 'mutated UI';
    const prepared = bindSourceDigest(pending, { source_host_id: 'host', result: { identity, request_digest: digest } });
    assert.equal(prepared.receipt, null); assert.equal(prepared.sent, false);
    assert.equal(prepared.request.messages && JSON.stringify(prepared.request.messages), JSON.stringify(request.messages));
    assert.deepEqual(sourceSubmissionBody(prepared), { ...request, request_digest: digest });
    assert.throws(() => sourceSubmissionBody({ ...prepared, sent: true }));
    assert.throws(() => exactSubmissionCancel(prepared));
});
test('cold persistence restores same identity, payload, digest and sent marker without redispatch', () => {
    const data = new Map<string, string>();
    const store = new SourceSubmissionStorage({ get length() { return data.size; }, key: index => [...data.keys()][index] ?? null, getItem: key => data.get(key) ?? null, setItem: (key, value) => { data.set(key, value); }, removeItem: key => { data.delete(key); } });
    const pending = { ...bound(), sent: true }; store.save(pending);
    const restored = store.read('host', 'client', 'conversation')!;
    assert.deepEqual(restored, pending); assert.throws(() => sourceSubmissionBody(restored));
    assert.deepEqual(sourceSubmissionQuery(restored), { operation: 'observe', identity, request_digest: digest });
    assert.equal(acceptSourceReceipt(restored, receipt('prepared')).receipt!.phase, 'prepared');
    assert.equal(store.read('host', 'other-client', 'conversation'), null);
});
test('only exact admitted receipt grants Run/root, replay is idempotent and cannot downgrade', () => {
    const admitted = acceptSourceReceipt(bound(), receipt('admitted'));
    assert.deepEqual(acceptSourceReceipt(admitted, receipt('admitted')), admitted);
    assert.deepEqual(exactSubmissionCancel(admitted), { identity, request_digest: digest });
    for (const phase of ['prepared', 'rejected', 'cancelled'] as const) assert.throws(() => acceptSourceReceipt(admitted, receipt(phase)));
    for (const changed of [{ source_host_id: 'other' }, { client_id: 'other' }, { user_message_id: 'other' }, { assistant_message_id: 'other' }])
        assert.throws(() => acceptSourceReceipt(bound(), { ...receipt('admitted'), identity: { ...identity, ...changed } }));
    assert.throws(() => acceptSourceReceipt(bound(), { ...receipt('admitted'), request_digest: 'c'.repeat(64) }));
    assert.throws(() => acceptSourceReceipt(bound(), { ...receipt('admitted'), admission: { ...admission, sequence: '0' } }));
});

test('late cleanup cannot remove a successor frozen request in the same conversation', () => {
    const data = new Map<string, string>();
    const store = new SourceSubmissionStorage({ get length() { return data.size; }, key: index => [...data.keys()][index] ?? null, getItem: key => data.get(key) ?? null, setItem: (key, value) => { data.set(key, value); }, removeItem: key => { data.delete(key); } });
    const previous = bound();
    const nextIdentity = { ...identity, submission_id: 'successor', user_message_id: 'successor-user', assistant_message_id: 'successor-assistant' };
    const nextRequest = { ...request, submission_id: nextIdentity.submission_id, user_message_id: nextIdentity.user_message_id, assistant_message_id: nextIdentity.assistant_message_id };
    const next = createSourceSubmission(nextIdentity, nextRequest); store.save(next); store.clear(previous);
    assert.deepEqual(store.read('host', 'client', 'conversation'), next);
});
test('pending cancel is a source tombstone request, not a speculative Run cancellation', () => {
    const pending = bound();
    assert.deepEqual(sourceSubmissionQuery(pending, true), { operation: 'cancel', identity, request_digest: digest });
    const cancelled = acceptSourceReceipt(pending, receipt('cancelled'));
    assert.equal(cancelled.receipt!.admission, null); assert.throws(() => exactSubmissionCancel(cancelled));
    assert.throws(() => acceptSourceReceipt(cancelled, receipt('admitted')));
    assert.throws(() => sourceSubmissionQuery(createSourceSubmission(identity, request)));
});
test('cold observation cannot borrow another source/client/root receipt', () => {
    const owner = { hostId: 'host', clientId: 'client', conversationId: 'conversation', generation: 1 };
    assert.equal(parseObservedSourceReceipt(receipt('admitted'), owner, 'submission', 'assistant').phase, 'admitted');
    assert.throws(() => parseObservedSourceReceipt(receipt('admitted'), { ...owner, clientId: 'other' }, 'submission', 'assistant'));
    assert.throws(() => parseObservedSourceReceipt(receipt('admitted'), owner, 'other', 'assistant'));
    assert.throws(() => parseObservedSourceReceipt(null, owner, 'submission', 'assistant'));
});
