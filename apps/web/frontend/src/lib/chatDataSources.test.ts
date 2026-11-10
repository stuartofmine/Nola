import test from 'node:test';
import assert from 'node:assert/strict';
import { confirmedConversationDataSource, conversationDataSourceUrl, conversationDataSourcesUrl, parseConversationDataSources } from './chatDataSources.ts';

const source = { id: 'source /?', name: '研究资料', kind: 'localFile', location: '/fixture', accessPolicy: 'conversationOptIn' };
const row = { source, conversationOverride: null, resolution: { enabled: false, usable: false, reason: 'conversation_opt_in_required' } };

test('conversation source URLs preserve opaque IDs and never take a server URL', () => {
    assert.equal(conversationDataSourcesUrl('conv /?'), '/api/data-sources?conversationId=conv%20%2F%3F');
    assert.equal(conversationDataSourceUrl('conv /?', source.id), '/api/chat/conversations/conv%20%2F%3F/data-sources/source%20%2F%3F');
});

test('source list and mutation receipts require exact conversation/source ownership and Core resolution', () => {
    assert.deepEqual(parseConversationDataSources({ conversationId: 'c', sources: [row] }, 'c')[0].conversationOverride, null);
    assert.throws(() => parseConversationDataSources({ conversationId: 'other', sources: [row] }, 'c'));
    assert.throws(() => parseConversationDataSources({ conversationId: 'c', sources: [row, row] }, 'c'));
    assert.throws(() => parseConversationDataSources({ conversationId: 'c', sources: [{ ...row, resolution: { enabled: true, usable: true, reason: 'future' } }] }, 'c'));
    assert.throws(() => parseConversationDataSources({ conversationId: 'c', sources: [{ ...row, resolution: { enabled: false, usable: true, reason: 'ready' } }] }, 'c'));
    assert.throws(() => parseConversationDataSources({ conversationId: 'c', sources: [{ ...row, resolution: { enabled: true, usable: false, reason: 'ready' } }] }, 'c'));
    assert.deepEqual(confirmedConversationDataSource({ source: { ...row, conversationOverride: true, resolution: { enabled: true, usable: true, reason: 'ready' } } }, source.id, true).conversationOverride, true);
    assert.throws(() => confirmedConversationDataSource({ source: row }, source.id, true));
    assert.throws(() => confirmedConversationDataSource({ source: row }, 'other', null));
});
