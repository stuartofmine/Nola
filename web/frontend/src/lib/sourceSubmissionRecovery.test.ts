import test from 'node:test';
import assert from 'node:assert/strict';
import { createSourceSubmissionRecovery } from './sourceSubmissionRecovery.ts';
import { SourceSubmissionStorage, createSourceSubmission, bindSourceDigest, acceptSourceReceipt } from './sourceSubmission.ts';

function fixture() {
    const data = new Map<string, string>();
    const storage = new SourceSubmissionStorage({ get length() { return data.size; }, key: index => [...data.keys()][index] ?? null,
        getItem: key => data.get(key) ?? null, setItem: (key, value) => { data.set(key, value); }, removeItem: key => { data.delete(key); } });
    const identity = { source_host_id: 'host', client_id: 'client', conversation_id: 'conversation',
        submission_id: 'run', user_message_id: 'user', assistant_message_id: 'assistant' };
    const request = { conversationId: 'conversation', expected_host_id: 'host', client_id: 'client', submission_id: 'run',
        user_message_id: 'user', assistant_message_id: 'assistant', messages: [{ role: 'user', content: 'private prompt' }] };
    const pending = { ...bindSourceDigest(createSourceSubmission(identity, request),
        { source_host_id: 'host', result: { identity, request_digest: 'a'.repeat(64) } }), sent: true };
    const receipt = (phase = 'admitted') => ({ protocol_version: 1, identity, request_digest: pending.request_digest, phase,
        admission: phase === 'admitted' ? { identity: { source_store_epoch: 'epoch', conversation_id: 'conversation', run_id: 'run', assistant_message_id: 'assistant' },
            sequence: '1', record_digest: 'b'.repeat(64) } : null, rejection_code: null });
    storage.save(pending);
    return { storage, pending, receipt };
}

test('receipt loss recovers by exact read-only queries, never replays the prompt or dispatches', async () => {
    const f = fixture(); const queries: Record<string, unknown>[] = []; let admitted = 0;
    const recovery = createSourceSubmissionRecovery({ storage: () => f.storage,
        observe: async body => {
            queries.push(body);
            return { source_host_id: 'host', result: queries.length === 1 ? null : f.receipt(queries.length === 2 ? 'prepared' : 'admitted') };
        }, admitted: () => { admitted++; }, wait: async () => {},
    });
    const first = recovery.watch(f.pending);
    assert.equal(first, recovery.watch(f.pending));
    await first;
    assert.equal(admitted, 1);
    assert.equal(queries.length, 3);
    for (const body of queries) assert.deepEqual(body, { operation: 'observe', identity: f.pending.identity, request_digest: f.pending.request_digest });
    assert.equal(f.storage.read('host', 'client', 'conversation')?.receipt?.phase, 'admitted');
});

test('foreign receipts remain uncertain until the exact source and digest confirm admission', async () => {
    const f = fixture(); let attempts = 0; let admitted = 0; const phases: unknown[] = [];
    const recovery = createSourceSubmissionRecovery({ storage: () => f.storage,
        observe: async () => {
            attempts++;
            return { source_host_id: attempts === 1 ? 'other' : 'host',
                result: attempts === 2 ? { ...f.receipt(), request_digest: 'c'.repeat(64) } : f.receipt() };
        }, admitted: () => { admitted++; },
        wait: async () => { phases.push(f.storage.read('host', 'client', 'conversation')?.receipt); },
    });
    await recovery.watch(f.pending);
    assert.deepEqual(phases, [null, null]); assert.equal(admitted, 1); assert.equal(attempts, 3);
});

test('a late read cannot revive deleted submissions, overwrite a successor, or escape workspace cancellation', async () => {
    for (const action of ['delete', 'replace', 'clear', 'forget'] as const) {
        const f = fixture(); let release!: (value: unknown) => void; let signal!: AbortSignal; let admitted = 0;
        const recovery = createSourceSubmissionRecovery({ storage: () => f.storage,
            observe: (_body, observedSignal) => { signal = observedSignal; return new Promise(resolve => { release = resolve; }); },
            admitted: () => { admitted++; },
        });
        const done = recovery.watch(f.pending);
        if (action === 'delete') f.storage.clear(f.pending);
        if (action === 'replace') f.storage.save({ ...f.pending, identity: { ...f.pending.identity, submission_id: 'next' },
            request: { ...f.pending.request, submission_id: 'next' } });
        if (action === 'clear') recovery.clear();
        if (action === 'forget') recovery.forget('conversation');
        release({ source_host_id: 'host', result: f.receipt() });
        await done;
        assert.equal(admitted, 0);
        assert.equal(signal.aborted, action === 'clear' || action === 'forget');
        assert.equal(f.storage.read('host', 'client', 'conversation')?.identity.submission_id,
            action === 'delete' ? undefined : action === 'replace' ? 'next' : 'run');
        assert.equal(f.storage.read('host', 'client', 'conversation')?.receipt ?? null, null);
    }
});

test('a live stream admission wins over an older prepared response; rejected and cancelled receipts stop observation', async () => {
    const f = fixture(); let admitted = 0;
    const recovery = createSourceSubmissionRecovery({ storage: () => f.storage, observe: async () => {
        f.storage.save(acceptSourceReceipt(f.pending, f.receipt()));
        return { source_host_id: 'host', result: f.receipt('prepared') };
    }, admitted: () => { admitted++; } });
    await recovery.watch(f.pending);
    assert.equal(admitted, 1);
    assert.equal(f.storage.read('host', 'client', 'conversation')?.receipt?.phase, 'admitted');
    for (const phase of ['rejected', 'cancelled']) {
        const g = fixture(); let calls = 0;
        const observer = createSourceSubmissionRecovery({ storage: () => g.storage,
            observe: async () => { calls++; return { source_host_id: 'host', result: g.receipt(phase) }; },
            admitted: () => { admitted++; },
        });
        await observer.watch(g.pending);
        assert.equal(calls, 1); assert.equal(admitted, 1);
        assert.equal(g.storage.read('host', 'client', 'conversation')?.receipt?.phase, phase);
    }
});
