import test from 'node:test';
import assert from 'node:assert/strict';
import {recallHits, recallUrl} from './chatRecall.ts';
const hit = {message_id: 'message', part_id: 'part', role: 'user', message_ordinal: 1, start_character: 0, snippet: '<img src=x>预算'};
const response = {conversationId: 'one', query: '预算', limit: 20, hits: [hit]};
test('recall URLs preserve exact opaque owner and multilingual literal query', () => {
    assert.equal(recallUrl('one/a', ' 预算%_ '), '/api/chat/conversations/one%2Fa/recall?q=%E9%A2%84%E7%AE%97%25_');
    assert.throws(() => recallUrl('one', ' '));
    assert.throws(() => recallUrl('one', 'a'.repeat(501)));
});
test('recall projection rejects foreign queries, owners, duplicate hits and unsafe offsets', () => {
    assert.deepEqual(recallHits(response, 'one', '预算'), [hit]);
    for (const value of [null, {...response, conversationId: 'two'}, {...response, query: 'old'}, {...response, limit: 200},
        {...response, hits: [hit, hit]}, {...response, hits: [{...hit, start_character: -1}]},
        {...response, hits: [{...hit, message_ordinal: Number.MAX_SAFE_INTEGER + 1}]}]) {
        assert.throws(() => recallHits(value, 'one', '预算'));
    }
});
