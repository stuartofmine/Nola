import test from 'node:test';
import assert from 'node:assert/strict';
import { createRecoveryDecision, parseRecoveryDecision, parseRecoveryInspection, recoverDecision, recoveryKey } from './unknownEffectRecovery.ts';
import type { RecoveryOwner, RecoveryInspection, RecoveryIntentStore } from './unknownEffectRecovery.ts';

const owner: RecoveryOwner = { expected_host_id: 'host', client_id: 'client', conversation_id: 'conversation', run_id: 'run', assistant_message_id: 'assistant' };
const inspected: RecoveryInspection = { conversation_id: 'conversation', run_id: 'run', assistant_message_id: 'assistant', invocation_config_digest: 'digest',
    independent_pending_effects: 0, retry_allowed: true, targets: [{ effect_id: 'effect', request_id: 'request', attempt_id: 'attempt', worker_generation: 'worker', lease_owner: 'lease-owner', lease_token: 'fence', expected_attempt_revision: 1 }] };
const wire = () => createRecoveryDecision(owner, inspected, 'authorize_risk_retry', 0, 'Explicitly accept repeated side effects', 'decision-request', 'resolution', 123);
const envelope = (result: unknown) => ({ source_host_id: 'host', owner, result });
const observed = (receipt: unknown) => envelope({ request_id: 'decision-request', receipt });
const committed = () => observed({ status: 'committed', result: { outcome: 'retry_authorized', run_id: 'run', resolution_id: 'resolution' } });
class MemoryStore implements RecoveryIntentStore {
    records = new Map<string, string>();
    async read(value: RecoveryOwner) { return this.records.get(recoveryKey(value)) ?? null; }
    async create(value: RecoveryOwner, body: string) { const key = recoveryKey(value); if (this.records.has(key)) throw new Error('exists'); this.records.set(key, body); }
    async clear(value: RecoveryOwner, body: string) { const key = recoveryKey(value); if (this.records.get(key) !== body) throw new Error('changed'); this.records.delete(key); }
}

test('inspection and persisted intent preserve exact owner, fences and UTF-8 limits', () => {
    assert.deepEqual(parseRecoveryInspection(envelope({ inspection: inspected }), owner), inspected);
    assert.equal(parseRecoveryInspection(envelope({ inspection: { ...inspected, targets: [], independent_pending_effects: 1 } }), owner), null);
    const value = parseRecoveryDecision(wire(), owner);
    assert.equal(value.input.targets[0].lease_token, 'fence');
    assert.throws(() => parseRecoveryInspection(envelope({ inspection: { ...inspected, run_id: 'other' } }), owner));
    assert.throws(() => parseRecoveryDecision(wire(), { ...owner, expected_host_id: 'other' }));
    assert.throws(() => parseRecoveryDecision(JSON.stringify({ ...value, input: { ...value.input, choice: { kind: 'verify_applied_receipt', reason: 'forged' } } }), owner));
    assert.throws(() => createRecoveryDecision(owner, inspected, 'authorize_risk_retry', 0, '理'.repeat(1366), 'request', 'resolution', 1));
    assert.throws(() => createRecoveryDecision(owner, { ...inspected, retry_allowed: false }, 'authorize_risk_retry', 0, 'retry', 'request', 'resolution', 1));
    assert.throws(() => createRecoveryDecision(owner, { ...inspected, independent_pending_effects: 1 }, 'keep_unknown_blocked', 0, 'stop', 'request', 'resolution', 1));
});

test('receipt-first resends exactly the frozen decision only when receipt is absent', async () => {
    const store = new MemoryStore(); const original = wire(); await store.create(owner, original);
    const sent: { operation: string; decision: unknown }[] = [];
    assert.equal(await recoverDecision(owner, original, store, async body => {
        const input = JSON.parse(body); sent.push(input);
        return input.operation === 'receipt' ? observed(null) : committed();
    }), 'retry_authorized');
    assert.deepEqual(sent.map(value => value.operation), ['receipt', 'resolve']);
    assert.deepEqual(sent[0].decision, sent[1].decision);
    assert.deepEqual(sent[1].decision, JSON.parse(original));
    assert.equal(await store.read(owner), null);
});

test('lost ACK survives reconstruction and adopts original receipt without another resolve', async () => {
    const store = new MemoryStore(); await store.create(owner, wire());
    await assert.rejects(recoverDecision(owner, wire(), store, async () => { throw new Error('offline'); }));
    const saved = await store.read(owner); assert.equal(saved, wire());
    const calls: string[] = [];
    assert.equal(await recoverDecision(owner, saved!, store, async body => {
        calls.push(JSON.parse(body).operation); return committed();
    }), 'retry_authorized');
    assert.deepEqual(calls, ['receipt']);
});

test('accepted or mismatched receipt never retires intent or creates a fresh decision', async () => {
    const store = new MemoryStore(); await store.create(owner, wire());
    let calls = 0;
    assert.equal(await recoverDecision(owner, wire(), store, async () => { calls++; return observed({ status: 'accepted' }); }), 'pending');
    assert.equal(calls, 1); assert.equal(await store.read(owner), wire());
    const wrong = observed({ status: 'committed', result: { outcome: 'retry_authorized', run_id: 'another-run', resolution_id: 'resolution' } });
    await assert.rejects(recoverDecision(owner, wire(), store, async () => wrong));
    assert.equal(await store.read(owner), wire());
});

test('different Runs retain separate decisions; competing same-Run decision cannot overwrite', async () => {
    const store = new MemoryStore(); await store.create(owner, wire());
    await assert.rejects(store.create(owner, wire()));
    const other = { ...owner, run_id: 'run-2' };
    const second = createRecoveryDecision(other, { ...inspected, run_id: 'run-2' }, 'keep_unknown_blocked', 0, 'stop', 'request-2', 'resolution-2', 124);
    await store.create(other, second);
    await recoverDecision(owner, wire(), store, async () => observed({ status: 'rejected' }));
    assert.equal(await store.read(owner), null); assert.equal(await store.read(other), second);
});
