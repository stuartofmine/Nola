import test from 'node:test';
import assert from 'node:assert/strict';
import { GoalControlIntentStore, dispatchGoalControlIntent, type GoalIntentLock } from './goalControlIntent.ts';

const isolatedLock: GoalIntentLock = async (_name, operation) => operation();
function storage() {
    const values = new Map<string, string>();
    return { getItem: (key: string) => values.get(key) ?? null,
        setItem: (key: string, value: string) => { values.set(key, value); }, removeItem: (key: string) => { values.delete(key); } };
}
const request = { operationId: 'original-operation', action: 'pause' as const, expectedGoalId: 'goal', expectedControlToken: 'observed-token' };
const receipt = { source_host_id: 'host', conversationId: 'conversation', operationId: request.operationId, authoritative: true,
    goalProjection: { applied_operations: { [request.operationId]: { goal_revision: 3, rejection: null } } } };

test('lost response retains exact intent across store recreation and retry', async () => {
    const disk = storage();
    const first = new GoalControlIntentStore(disk, 'host');
    await assert.rejects(dispatchGoalControlIntent('conversation', request, async () => { throw new Error('lost acknowledgement'); }, first, isolatedLock));
    const reopened = new GoalControlIntentStore(disk, 'host');
    assert.deepEqual(reopened.read('conversation'), request);
    await dispatchGoalControlIntent('conversation', reopened.read('conversation')!, async (_url, options) => {
        assert.deepEqual(JSON.parse(options.body), { ...request, expectedHostId: 'host' });
        return receipt;
    }, reopened, isolatedLock);
    assert.equal(reopened.read('conversation'), null);
});

test('pending admission cannot clear intent and another action cannot replace it', async () => {
    const store = new GoalControlIntentStore(storage(), 'host');
    await dispatchGoalControlIntent('conversation', request, async () => ({ source_host_id: 'host', conversationId: 'conversation',
        operationId: request.operationId, authoritative: false, controlPending: true }), store, isolatedLock);
    assert.deepEqual(store.read('conversation'), request);
    let dispatched = false;
    await assert.rejects(dispatchGoalControlIntent('conversation', { ...request, operationId: 'replacement' },
        async () => { dispatched = true; return receipt; }, store, isolatedLock));
    assert.equal(dispatched, false);
});

test('foreign receipt and failed durable save cannot lose or send an intent', async () => {
    const store = new GoalControlIntentStore(storage(), 'host');
    await assert.rejects(dispatchGoalControlIntent('conversation', request, async () => ({ ...receipt, conversationId: 'foreign' }), store, isolatedLock));
    assert.deepEqual(store.read('conversation'), request);
    const broken = new GoalControlIntentStore({ ...storage(), setItem: () => { throw new Error('quota'); } }, 'host');
    let dispatched = false;
    await assert.rejects(dispatchGoalControlIntent('conversation', request, async () => { dispatched = true; return receipt; }, broken, isolatedLock));
    assert.equal(dispatched, false);
});

test('settling an old response cannot remove a different persisted intent', () => {
    const store = new GoalControlIntentStore(storage(), 'host');
    store.save('conversation', { ...request, operationId: 'new-operation' });
    assert.throws(() => store.settle('conversation', request, receipt));
    assert.equal(store.read('conversation')?.operationId, 'new-operation');
});

test('a different Host at the same browser origin cannot recover the old intent', () => {
    const disk = storage();
    const original = new GoalControlIntentStore(disk, 'original-host');
    original.save('conversation', request);
    const replacement = new GoalControlIntentStore(disk, 'replacement-host');
    assert.equal(replacement.read('conversation'), null);
    assert.deepEqual(original.read('conversation'), request);
});

test('concurrent tabs retain the first unknown operation and never dispatch a replacement', async () => {
    const disk = storage();
    let tail = Promise.resolve();
    const serial: GoalIntentLock = (_name, operation) => {
        const result = tail.then(operation);
        tail = result.then(() => {}, () => {});
        return result;
    };
    let release!: () => void;
    const barrier = new Promise<void>(resolve => { release = resolve; });
    const calls: string[] = [];
    const first = dispatchGoalControlIntent('conversation', request, async () => {
        calls.push('first'); await barrier; throw new Error('response lost');
    }, new GoalControlIntentStore(disk, 'host'), serial);
    const second = dispatchGoalControlIntent('conversation', { ...request, operationId: 'replacement' }, async () => {
        calls.push('replacement'); return receipt;
    }, new GoalControlIntentStore(disk, 'host'), serial);
    release();
    const results = await Promise.allSettled([first, second]);
    assert.deepEqual(results.map(value => value.status), ['rejected', 'rejected']);
    assert.deepEqual(calls, ['first']);
    assert.deepEqual(new GoalControlIntentStore(disk, 'host').read('conversation'), request);
});

test('invalid durable identity never replaces intent or reaches transport', async () => {
    const store = new GoalControlIntentStore(storage(), 'host');
    let sent = false;
    await assert.rejects(dispatchGoalControlIntent('conversation', { ...request, operationId: ' padded ' },
        async () => { sent = true; return receipt; }, store, isolatedLock));
    assert.equal(sent, false);
    assert.equal(store.read('conversation'), null);
    assert.throws(() => store.save('conversation', { ...request, unexpected: 'field' } as typeof request));
    assert.equal(store.read('conversation'), null);
    assert.throws(() => store.save('conversation', { action: 'start', operationId: 'start', objective: ' padded objective ' }));
    assert.equal(store.read('conversation'), null);
});
