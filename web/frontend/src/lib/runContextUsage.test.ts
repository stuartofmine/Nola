import test from 'node:test';
import assert from 'node:assert/strict';
import { RunContextUsageProjection, contextUsageDigest, parseRunContextUsage, readHistoricalRunContextUsage } from './runContextUsage.ts';
import type { CommittedRunContextUsage } from './runContextUsage.ts';

const owner = { hostId: 'host', clientId: 'client', conversationId: 'conversation', generation: 1 };
const identity = { source_store_epoch: 'epoch', conversation_id: 'conversation', run_id: 'run', assistant_message_id: 'root' };
const admission = { identity, sequence: '2', record_digest: 'a'.repeat(64) };
test('cache facts survive verified restoration and reject fabricated hit rates', async () => {
    const value = await record();
    value.usage.cache_usage = { input_tokens: 1000, read_tokens: 800, write_tokens: 100, hit_rate_bps: 8000, is_compaction: true };
    value.record_digest = await contextUsageDigest(value);
    const projection = new RunContextUsageProjection(); projection.bind(owner, admission);
    assert.deepEqual((await projection.accept(value, owner, admission, () => true))?.usage.cache_usage, value.usage.cache_usage);
    const broken = structuredClone(value); broken.usage.cache_usage!.hit_rate_bps = 9500;
    assert.throws(() => parseRunContextUsage(broken, admission), /cache rate/);
    value.usage.cache_usage = { input_tokens: 1000, read_tokens: null, write_tokens: 1000, hit_rate_bps: null, is_compaction: false };
    assert.equal(parseRunContextUsage(value, admission).usage.cache_usage?.hit_rate_bps, null);
});

test('estimated occupancy and persisted cache diagnostics retain distinct request identities', async () => {
    const value = await record();
    value.usage.measurement = { request_id: 'next-request', attempt_id: null, context_generation: 4 };
    value.usage.estimated = true;
    value.usage.prompt_cache_measurement = { request_id: 'paid-request', attempt_id: null, context_generation: 3 };
    value.usage.prompt_cache_diagnostic = { schemaVersion: 1, status: 'hit', identityStable: false,
        reasons: ['tool_manifest_changed'], fragmentChanges: [{ fragmentId: 'tools', kind: 'changed' }],
        cacheReadInputTokens: 800, cacheWriteInputTokens: null };
    value.record_digest = await contextUsageDigest(value);
    const projection = new RunContextUsageProjection(); projection.bind(owner, admission);
    const restored = await projection.accept(JSON.parse(JSON.stringify(value)), owner, admission, () => true);
    assert.equal(restored?.usage.measurement?.request_id, 'next-request');
    assert.equal(restored?.usage.prompt_cache_measurement?.request_id, 'paid-request');
    assert.deepEqual(restored?.usage.prompt_cache_diagnostic, value.usage.prompt_cache_diagnostic);
    assert.throws(() => parseRunContextUsage({ ...value, protocol_version: 2 }, admission));
    assert.throws(() => parseRunContextUsage({ ...value, usage: { ...value.usage, prompt_cache_measurement: null } }, admission), /source request/);
});
async function record(event = '9007199254740993', used = '64000', at = admission): Promise<CommittedRunContextUsage> {
    const value = { protocol_version: 1 as const, identity: at.identity, admission: at, event_sequence: event,
        attempt_sequence: '0', run_cache_usage: null,
        usage: { measurement: null, prompt_cache_measurement: null, prompt_cache_diagnostic: null,
            used_tokens: used, context_window_tokens: '128000', prompt_token_limit: '120000', estimated: false } };
    return { ...value, record_digest: await contextUsageDigest(value) };
}
test('committed usage advances lossless events, ignores stale records and rejects same-event forks', async () => {
    const projection = new RunContextUsageProjection(); projection.bind(owner, admission);
    const actual = await record();
    assert.deepEqual(await projection.accept(actual, owner, admission, () => true), actual);
    assert.deepEqual(await projection.accept(actual, owner, admission, () => true), actual);
    assert.deepEqual(await projection.accept(await record('5', '1'), owner, admission, () => true), actual);
    await assert.rejects(projection.accept(await record(actual.event_sequence, '64001'), owner, admission, () => true), /fork/);
    assert.deepEqual(projection.record, actual);
    await assert.rejects(projection.accept({ ...actual, record_digest: 'b'.repeat(64) }, owner, admission, () => true), /digest/);
});

test('request totals advance without a context event and reject crossed frontiers', async () => {
    const initial = await record();
    const projection = new RunContextUsageProjection(); projection.bind(owner, admission);
    await projection.accept(initial, owner, admission, () => true);
    const next: CommittedRunContextUsage = { ...initial, attempt_sequence: '4', run_cache_usage: {
        input_tokens: 2000, read_tokens: 1500, write_tokens: 100, hit_rate_bps: 7500,
        is_compaction: false, reported_requests: 2, unreported_requests: 1,
    } };
    next.record_digest = await contextUsageDigest(next);
    assert.deepEqual(await projection.accept(next, owner, admission, () => true), next);
    const crossed = { ...next, event_sequence: '5', attempt_sequence: '6' };
    crossed.record_digest = await contextUsageDigest(crossed);
    await assert.rejects(projection.accept(crossed, owner, admission, () => true), /frontier fork/);
    assert.deepEqual(projection.record, next);
    const tampered = structuredClone(next); tampered.run_cache_usage!.reported_requests++;
    tampered.record_digest = await contextUsageDigest(tampered);
    await assert.rejects(projection.accept(tampered, owner, admission, () => true), /attempt fork/);
});
test('foreign source/client/root/store and invalid admission do not acquire the visible scope', async () => {
    const actual = await record(); const projection = new RunContextUsageProjection(); projection.bind(owner, admission);
    for (const changed of [{ run_id: 'other' }, { assistant_message_id: 'child' }, { source_store_epoch: 'other' }, { conversation_id: 'other' }])
        assert.throws(() => parseRunContextUsage({ ...actual, identity: { ...identity, ...changed } }, admission));
    assert.throws(() => projection.bind({ ...owner, hostId: 'other' }, admission));
    assert.throws(() => projection.bind({ ...owner, clientId: 'other' }, admission));
    assert.equal(await projection.accept(actual, { ...owner, generation: 0 }, admission, () => true), null);
    assert.throws(() => parseRunContextUsage({ ...actual, admission: { ...admission, record_digest: 'b'.repeat(64) } }, admission));
    assert.throws(() => parseRunContextUsage({ ...actual, event_sequence: '18446744073709551616' }, admission));
    assert.equal(projection.record, null);
});
test('new Run reset and generation changes discard late hash completion and old history', async () => {
    const actual = await record(); const projection = new RunContextUsageProjection(); projection.bind(owner, admission);
    let current = true;
    const pending = projection.accept(actual, owner, admission, () => current);
    current = false; assert.equal(await pending, null); assert.equal(projection.record, null);
    const successor = { ...admission, identity: { ...identity, run_id: 'successor', assistant_message_id: 'successor-root' } };
    projection.bind(owner, successor);
    assert.equal(await projection.accept(actual, owner, admission, () => true), null);
    assert.equal(projection.bind(owner, admission, true), false, 'older history cannot replace a live newer Run');
    projection.reset(); projection.bind({ ...owner, generation: 2 }, admission);
    assert.equal(await projection.accept(actual, owner, admission, () => true), null);
});
test('cold history uses the actual sealed root and only reads that exact committed usage', async () => {
    const actual = await record(); const projection = new RunContextUsageProjection(); const calls: Record<string, unknown>[] = [];
    const member = { ...admission, identity: { ...identity, assistant_message_id: 'successor-assistant-in-same-run' } };
    const transport = async (body: Record<string, unknown>) => {
        calls.push(body);
        return { source_host_id: 'host', result: body.operation === 'terminal_seal'
            ? { protocol_version: 1, identity, admission, seal_digest: 'c'.repeat(64), result: {
                result_id: 'result', conversation_id: 'conversation', run_id: 'run', originating_submission_id: 'run',
                input: { watermark: '1', applications_digest: 'd'.repeat(64) }, decided_at_revision: '2',
                decision_event_cursor: '3', requirement_revision: '1', evidence_frontier: '2',
                value: { kind: 'answered', answer_part_refs: ['answer'] },
            } } : actual };
    };
    assert.deepEqual(await readHistoricalRunContextUsage(owner, member, projection, transport, () => true), actual);
    assert.deepEqual(calls, [{ operation: 'terminal_seal', conversation_id: 'conversation', run_id: 'run' },
        { operation: 'context_usage', identity }]);
    const before = projection.record;
    await assert.rejects(readHistoricalRunContextUsage(owner, member, new RunContextUsageProjection(), async () =>
        ({ source_host_id: 'foreign', result: null }), () => true), /source/);
    assert.deepEqual(projection.record, before);
});
