import test from 'node:test';
import assert from 'node:assert/strict';
import {memoryRecords, memoryView, memoryDeleteUrl, memoryDeletionConfirmed, memoryDraft, memoryWriteReceipt} from './memoryCenter.ts';
const old = {id: 'old', scope: 'global', kind: 'preference', content: '旧偏好', summary: null, source: 'user', tags: [], confidence: 1, supersedes: [], importance: 0.7, sensitivity: 'private', expires_at: null, created_at: '', updated_at: ''};
const next = {...old, id: 'new', content: '新偏好', supersedes: ['old']};
test('revision drafts preserve metadata and require matching content and lineage receipts', () => {
    const draft = memoryDraft(old); draft.content = next.content;
    assert.equal(draft.importance, 0.7); assert.equal(draft.sensitivity, 'private');
    assert.equal(memoryWriteReceipt({type: 'memory_revised', previous_id: 'old', record: next}, draft, 'old').id, 'new');
    for (const response of [{type: 'memory_created', record: next}, {type: 'memory_revised', previous_id: 'other', record: next},
        {type: 'memory_revised', previous_id: 'old', record: {...next, supersedes: []}}]) {
        assert.throws(() => memoryWriteReceipt(response, draft, 'old'));
    }
});
test('memory deletion requires a typed receipt for the exact opaque identity', () => {
    assert.equal(memoryDeleteUrl('a/b?'), '/api/memory/a%2Fb%3F');
    assert.equal(memoryDeletionConfirmed({type: 'memory_deleted', id: 'one'}, 'one'), true);
    for (const value of [null, {}, {ok: true}, {type: 'memory_deleted', id: 'other'}]) {
        assert.equal(memoryDeletionConfirmed(value, 'one'), false);
    }
});
test('filtering memories never hides revision evidence when marking superseded records', () => {
    const records = memoryRecords({type: 'memory_records', limit: 2000, records: [old, next]});
    const filtered = memoryView(records, '旧', 'zh-Hans');
    assert.equal(filtered.length, 1);
    assert.equal(filtered[0].superseded, true);
    assert.equal(memoryView(records, '新', 'zh-Hans')[0].superseded, false);
});
test('memory list rejects duplicate identities, unknown envelopes and invalid confidence', () => {
    for (const value of [{}, {type: 'memory_records', limit: 2000, records: [old, old]},
        {type: 'memory_records', limit: 2000, records: [{...old, confidence: 2}]},
        {type: 'memory_records', limit: 2000, records: [{...old, supersedes: [null]}]}]) {
        assert.throws(() => memoryRecords(value));
    }
});
