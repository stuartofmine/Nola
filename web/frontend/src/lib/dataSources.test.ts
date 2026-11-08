import { test } from 'node:test';
import assert from 'node:assert/strict';
import { newSourceId, parseSources, parseSource, parseUsage, sourceDraft, sourcePayload, sourceReceipt, deletionReceipt, sourceState, disclosureKey, replaceSource } from './dataSources.ts';
import type { DataSource } from './dataSources.ts';
import { latestRequest } from './latestRequest.ts';

test('new source IDs do not require secure-context-only randomUUID', () => {
    const random = { getRandomValues: crypto.getRandomValues.bind(crypto) };
    const ids = Array.from({ length: 20 }, () => newSourceId(random));
    assert.equal(new Set(ids).size, ids.length);
    for (const id of ids) assert.match(id, /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
});

function source(id = 'source-1'): DataSource {
    return { id, name: 'Private notes', kind: 'localFile', location: '/fixtures/notes.json', accessPolicy: 'blocked',
        validationContract: { sourceId: id, readerDescription: 'Reader', schemaFields: ['title'], redactedSample: '[redacted]', recordCount: 2, maximumAgeMinutes: 60, validationState: 'verified', validatedAt: '2026-09-22T00:00:00Z' } };
}
test('source drafts preserve restrictive and unknown policies until an explicit valid selection', () => {
    const original = source(); const draft = sourceDraft(original);
    assert.equal(draft.accessPolicy, 'blocked');
    draft.name = 'Edited'; assert.equal(original.name, 'Private notes');
    assert.equal(sourcePayload(draft).accessPolicy, 'blocked');
    assert.equal(sourceDraft().accessPolicy, 'conversationOptIn');
    const unknown = sourceDraft({ ...original, accessPolicy: '__proto__', kind: 'future' });
    assert.equal(unknown.accessPolicy, '__proto__');
    assert.throws(() => sourcePayload(unknown));
    for (const maximumAgeMinutes of ['', '0', '1.5', 'NaN', '10081', 'Infinity']) assert.throws(() => sourcePayload({ ...draft, maximumAgeMinutes }));
    assert.equal(sourcePayload({ ...draft, maximumAgeMinutes: '10080' }).maximumAgeMinutes, 10080);
    assert.throws(() => sourcePayload({ ...draft, name: ' ' }));
});
test('malformed or duplicate source and usage reads are errors, not empty catalogs', () => {
    const original = source();
    const rows = parseSources({ sources: [{ source: original }] });
    rows[0].validationContract!.schemaFields.push('changed');
    assert.deepEqual(original.validationContract!.schemaFields, ['title']);
    for (const value of [{}, { sources: null }, { sources: [{}] }, { sources: [{ source: original }, { source: original }] }]) assert.throws(() => parseSources(value));
    assert.throws(() => parseSource({ ...original, validationContract: { ...original.validationContract, sourceId: 'another-source' } }));
    assert.throws(() => parseUsage({ usageRecords: [{}] }));
    assert.deepEqual(parseUsage({ usageRecords: [] }), []);
});
test('mutations require exact tested and deletion receipts before updating the catalog', () => {
    const original = source(); const next = { ...original, name: 'New title' };
    assert.throws(() => sourceReceipt({ source: next, tested: false }, original.id));
    assert.throws(() => sourceReceipt({ source: next, tested: true }, 'other'));
    assert.throws(() => sourceReceipt({}));
    assert.throws(() => deletionReceipt({ deleted: true, id: 'other' }, original.id));
    assert.throws(() => deletionReceipt({ deleted: false, id: original.id }, original.id));
    deletionReceipt({ deleted: true, id: original.id }, original.id);
    const updated = replaceSource([original, source('other')], sourceReceipt({ source: next, tested: true }, original.id));
    assert.deepEqual(updated.map(item => item.id), [original.id, 'other']);
    assert.equal(updated[0].name, 'New title'); assert.equal(original.name, 'Private notes');
});
test('missing disclosure is unknown and validation display never changes access policy', () => {
    const record = { id: 'use', sourceId: 'source', sourceTitle: 'Title', consumer: 'model', purpose: 'Context', usedAt: '2026-09-22T00:00:00Z' };
    for (const value of [undefined, null]) assert.equal(disclosureKey(parseUsage({ usageRecords: [{ ...record, disclosedToCloud: value }] })[0]), 'data_sources.usage.unknown');
    assert.equal(disclosureKey({ ...record, disclosedToCloud: false }), 'data_sources.usage.local');
    assert.equal(disclosureKey({ ...record, disclosedToCloud: true }), 'data_sources.usage.cloud');
    assert.throws(() => parseUsage({ usageRecords: [{ ...record, disclosedToCloud: 'false' }] }));
    const blocked = source(); assert.equal(sourceState(blocked), 'blocked'); assert.equal(blocked.accessPolicy, 'blocked');
    assert.equal(sourceState({ ...blocked, accessPolicy: 'conversationOptIn', validationContract: null }), 'not_tested');
});
test('a pre-mutation source read cannot overwrite a receipt even when abort is ignored', async () => {
    const reads = latestRequest<DataSource[]>();
    let resolve!: (value: DataSource[]) => void;
    const pending = reads.run(() => new Promise<DataSource[]>(done => { resolve = done; }));
    reads.cancel();
    const receipt = sourceReceipt({ source: { ...source(), name: 'Saved' }, tested: true }, 'source-1');
    const catalog = replaceSource([source()], receipt);
    resolve([source()]);
    assert.equal(await pending, undefined);
    assert.equal(catalog[0].name, 'Saved');
});
