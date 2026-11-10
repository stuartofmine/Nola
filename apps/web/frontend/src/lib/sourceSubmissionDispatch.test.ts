import test from 'node:test';
import assert from 'node:assert/strict';
import { createSourceSubmission, SourceSubmissionStorage } from './sourceSubmission.ts';
import { dispatchSourceSubmission, SourceSubmissionNotDispatchedError, journalEventQueue } from './sourceSubmissionDispatch.ts';

function fixture() {
    const data = new Map<string, string>();
    const storage = new SourceSubmissionStorage({get length() { return data.size; }, key: index => [...data.keys()][index] ?? null, getItem: key => data.get(key) ?? null, setItem: (key, value) => {data.set(key, value);}, removeItem: key => {data.delete(key);}});
    const identity = {source_host_id: 'host', client_id: 'client', conversation_id: 'conversation', submission_id: 'submission', user_message_id: 'user', assistant_message_id: 'assistant'};
    const pending = createSourceSubmission(identity, {conversationId: 'conversation', expected_host_id: 'host', client_id: 'client', submission_id: 'submission', user_message_id: 'user', assistant_message_id: 'assistant', messages: [{role: 'user', content: 'original'}]});
    storage.save(pending);
    const prepared = {source_host_id: 'host', result: {identity, request_digest: 'a'.repeat(64)}};
    const receipt = {protocol_version: 1, identity, request_digest: 'a'.repeat(64), phase: 'prepared', admission: null};
    return {storage, pending, prepared, receipt, signal: new AbortController().signal};
}
test('physical dispatch waits for durable bound ticket and respects deletion during persistence', async () => {
    const f = fixture();
    const controller = new AbortController();
    let persisted!: () => void;
    let bound!: () => void;
    const entered = new Promise<void>(resolve => { bound = resolve; });
    let sends = 0;
    const dispatched = dispatchSourceSubmission(f.pending, {...f, signal: controller.signal,
        prepare: async () => f.prepared,
        bound: async () => { bound(); await new Promise<void>(resolve => { persisted = resolve; }); },
        stream: async function* () { sends++; },
    });
    await entered;
    assert.equal(sends, 0);
    controller.abort(); persisted();
    await assert.rejects(dispatched);
    assert.equal(sends, 0);
});

test('failed durable ticket update never opens a physical stream', async () => {
    const f = fixture();
    await assert.rejects(dispatchSourceSubmission(f.pending, {...f,
        prepare: async () => f.prepared,
        bound: async () => { throw new Error('disk unavailable'); },
        stream: async function* () { assert.fail('unpersisted dispatch'); },
    }));
    assert.equal(f.storage.read('host', 'client', 'conversation')?.sent, true);
});
test('failed or invalid preparation retires only the unsent intent and permits a subsequent submission', async () => {
    for (const malformed of [false, true]) {
        const f = fixture();
        await assert.rejects(dispatchSourceSubmission(f.pending, {...f,
            prepare: async () => { if (malformed) return {}; throw new Error('offline'); },
            stream: async function* () { assert.fail('must not dispatch'); },
        }), SourceSubmissionNotDispatchedError);
        assert.equal(f.storage.read('host', 'client', 'conversation'), null);
        f.storage.save(f.pending);
        let sends = 0;
        await dispatchSourceSubmission(f.pending, {...f, prepare: async () => f.prepared,
            stream: async function* () { sends++; },
        });
        assert.equal(sends, 1);
    }
});
test('a failed overlapping preparer cannot retire an already dispatched intent', async () => {
    const f = fixture();
    let reject!: (error: Error) => void;
    const failed = dispatchSourceSubmission(f.pending, {...f,
        prepare: () => new Promise((_, fail) => { reject = fail; }),
        stream: async function* () { assert.fail('late dispatch'); },
    });
    await dispatchSourceSubmission(f.pending, {...f, prepare: async () => f.prepared, stream: async function* () {}});
    reject(new Error('offline'));
    await assert.rejects(failed, error => !(error instanceof SourceSubmissionNotDispatchedError));
    assert.equal(f.storage.read('host', 'client', 'conversation')?.sent, true);
});
test('original dispatch survives observer removal and persists receipts for a remounted page', async () => {
    const f = fixture();
    let release!: (value: unknown) => void;
    let mounted = true, events = 0, sends = 0;
    const done = dispatchSourceSubmission(f.pending, {...f, prepare: () => new Promise(resolve => {release = resolve;}),
        stream: async function* (body) { sends++; assert.equal(body.request_digest, f.prepared.result.request_digest); yield {type: 'submission_receipt', receipt: f.receipt}; },
        event: async () => {if (mounted) events++;}});
    mounted = false;
    release(f.prepared);
    await done;
    assert.equal(sends, 1); assert.equal(events, 0);
    assert.equal(f.storage.read('host', 'client', 'conversation')?.receipt?.phase, 'prepared');
    await dispatchSourceSubmission(f.pending, {...f, prepare: async () => assert.fail('reprepare'), stream: async function* () {assert.fail('redispatch');}});
});
test('cancelling or replacing the persisted intent during prepare prevents its dispatch', async () => {
    for (const replacement of [false, true]) {
        const f = fixture();
        let release!: (value: unknown) => void;
        const done = dispatchSourceSubmission(f.pending, {...f, prepare: () => new Promise(resolve => {release = resolve;}), stream: async function* () {assert.fail('late dispatch');}});
        f.storage.clear(f.pending);
        if (replacement) f.storage.save(createSourceSubmission({...f.pending.identity, submission_id: 'next'}, {...f.pending.request, submission_id: 'next'}));
        release(f.prepared); await done;
        assert.equal(f.storage.read('host', 'client', 'conversation')?.identity.submission_id, replacement ? 'next' : undefined);
    }
});
test('lost stream response persists sent marker and never retries; abort prevents dispatch', async () => {
    const f = fixture();
    await assert.rejects(dispatchSourceSubmission(f.pending, {...f, prepare: async () => f.prepared, stream: async function* () {throw new Error('lost ACK');}}));
    assert.equal(f.storage.read('host', 'client', 'conversation')?.sent, true);
    await dispatchSourceSubmission(f.pending, {...f, prepare: async () => assert.fail(), stream: async function* () {assert.fail();}});
    const other = fixture(); const controller = new AbortController(); controller.abort();
    await assert.rejects(dispatchSourceSubmission(other.pending, {...other, signal: controller.signal, prepare: async () => assert.fail(), stream: async function* () {assert.fail();}}));
});
test('two overlapping preparers cannot both dispatch the original intent', async () => {
    const f = fixture(); let sends = 0;
    const options = {...f, prepare: async () => f.prepared, stream: async function* () {sends++;}};
    await Promise.all([dispatchSourceSubmission(f.pending, options), dispatchSourceSubmission(f.pending, options)]);
    assert.equal(sends, 1);
});
test('a foreign receipt cannot acquire authority after navigation', async () => {
    const f = fixture();
    await assert.rejects(dispatchSourceSubmission(f.pending, {...f, prepare: async () => f.prepared,
        onReceipt: () => assert.fail('foreign receipt reached workspace observers'),
        stream: async function* () {yield {type: 'submission_receipt', receipt: {...f.receipt, identity: {...f.receipt.identity, conversation_id: 'other'}}};}}));
    assert.equal(f.storage.read('host', 'client', 'conversation')?.receipt, null);
});
test('validated persisted receipts reach workspace observers even without a mounted page', async () => {
    const f = fixture(); let receipts = 0;
    await dispatchSourceSubmission(f.pending, { ...f, prepare: async () => f.prepared,
        stream: async function* () { yield { type: 'submission_receipt', receipt: f.receipt }; },
        onReceipt: value => {
            receipts++;
            assert.deepEqual(value, f.storage.read('host', 'client', 'conversation'));
            assert.deepEqual(value.receipt, { ...f.receipt, rejection_code: null });
        },
    });
    assert.equal(receipts, 1);
});

test('paused journal hydration does not delay permission frames and remains ordered', async () => {
    let release!: () => void; const paused = new Promise<void>(resolve => {release=resolve;});
    const events: string[] = [];
    const queue = journalEventQueue(async event => {
        if (event.type === 'assistant_journal_committed_v1') await paused;
        events.push(String(event.id));
    });
    await queue.push({type:'assistant_journal_committed_v1',id:'first'});
    await queue.push({type:'assistant_journal_committed_v1',id:'second'});
    await queue.push({type:'permission_required',id:'permission'});
    await queue.push({type:'assistant_run_terminal_committed_v1',id:'terminal'});
    assert.deepEqual(events,['permission','terminal']);
    release(); await queue.flush();
    assert.deepEqual(events,['permission','terminal','first','second']);
});
