import test from 'node:test';
import assert from 'node:assert/strict';
import { parseChatConversation, parseChatConversations, sameConversationOperation, conversationUrl, messageWindowUrl, conversationTitleRequest, conversationBranchRequest, conversationTruncateRequest, createConversationDraftStore, parseConversationExport } from './chatConversations.ts';
import { chatConversationMessages } from './locales/chatConversations.ts';
import { conversationPinRequest, conversationPinConfirmed } from './chatConversations.ts';

test('pinning requires exact boolean state and revision-bound receipt', () => {
    const request = conversationPinRequest({ configurationRevision: 4, isPinned: false }, false);
    assert.deepEqual(request, { expectedRevision: 4, isPinned: true });
    assert.equal(conversationPinConfirmed({ configurationRevision: 5, isPinned: true }, request), true);
    for (const receipt of [{ configurationRevision: 4, isPinned: true }, { configurationRevision: 5, isPinned: false }, { ok: true }]) assert.equal(conversationPinConfirmed(receipt, request), false);
    for (const configuration of [{}, { configurationRevision: 0, isPinned: false }, { configurationRevision: 4, isPinned: true }, { configurationRevision: 4, isPinned: 'false' }]) assert.throws(() => conversationPinRequest(configuration, false));
    assert.throws(() => parseChatConversation({ ...conversation, isPinned: undefined }));
});
const conversation = { revision: 3, configurationRevision: 7, isPinned: false, conversationId: 'conv: /?#', title: '研究', selectedModelSource: 'cloud', selectedModelId: 'model-a', executionBackend: 'nola', createdAt: '2026-09-23', updatedAt: '2026-09-23' };
test('CLI camelCase list parses without exposing unrelated stored context', () => {
    assert.deepEqual(parseChatConversation({ ...conversation, sessionMemory: 'private context' }), conversation);
    assert.deepEqual(parseChatConversations({ conversations: [conversation] }), [conversation]);
    assert.throws(() => parseChatConversations({ conversations: [conversation, conversation] }));
    assert.throws(() => parseChatConversations({ conversations: [{ ...conversation, conversationId: '' }] }));
    assert.throws(() => parseChatConversations({ conversations: [{ ...conversation, selectedModelSource: 3 }] }));
});
test('routes encode opaque conversation identity and validate actual ordinal pagination', () => {
    assert.equal(conversationUrl(conversation.conversationId), '/api/chat/conversations/conv%3A%20%2F%3F%23');
    assert.equal(messageWindowUrl('a'), '/api/chat/conversations/a/messages?limit=48');
    assert.equal(messageWindowUrl('a', 20, 200, { projectionRevision: 5, eventSequence: 7 }), '/api/chat/conversations/a/messages?limit=200&before=20&expectedRevision=5&expectedEventSequence=7');
    assert.throws(() => messageWindowUrl('a', 20, 200));
    assert.throws(() => messageWindowUrl('a', 20, 200, { projectionRevision: -1, eventSequence: 7 }));
    for (const limit of [0, 201, 1.5, NaN]) assert.throws(() => messageWindowUrl('a', null, limit));
    assert.throws(() => messageWindowUrl('a', 1.5));
    assert.throws(() => conversationUrl(' '));
});
test('mutations send sparse real backend field names, truncate explicitly keeps selected message', () => {
    assert.deepEqual(conversationTitleRequest('  new title  ', 3), { title: 'new title', expectedRevision: 3 });
    assert.deepEqual(conversationBranchRequest('opaque message'), { messageId: 'opaque message' });
    assert.deepEqual(conversationBranchRequest(null), { messageId: '' });
    assert.deepEqual(conversationTruncateRequest('opaque message'), { messageId: 'opaque message', keepMessage: true });
    assert.throws(() => conversationTitleRequest(' ', 3));
    assert.throws(() => conversationTitleRequest('title', 0));
    assert.throws(() => conversationBranchRequest(''));
    assert.throws(() => conversationTruncateRequest(''));
});
test('owner generation rejects stale A→B→A read completions', () => {
    const first = { conversationId: 'a', generation: 1 };
    assert.equal(sameConversationOperation(first, { ...first }), true);
    assert.equal(sameConversationOperation(first, { conversationId: 'b', generation: 1 }), false);
    assert.equal(sameConversationOperation(first, { conversationId: 'a', generation: 3 }), false);
});
test('draft cache isolates exact conversation IDs and has explicit non-persistent lifetime', () => {
    const drafts = createConversationDraftStore();
    drafts.write('a', '  未发送\n'); drafts.write('b', 'draft B');
    assert.equal(drafts.read('a'), '  未发送\n'); assert.equal(drafts.read('b'), 'draft B');
    drafts.delete('a'); assert.equal(drafts.read('a'), ''); assert.equal(drafts.read('b'), 'draft B');
    assert.equal(createConversationDraftStore().read('b'), '');
    drafts.clear(); assert.equal(drafts.read('b'), '');
});
test('export is a validated opaque download rather than HTML rendering', () => {
    const value = parseConversationExport({ filename: '../test\u0000.html', mimeType: 'text/html', content: '<script>not executed</script>' });
    assert.equal(value.filename.includes('/'), false); assert.equal(value.filename.includes('\0'), false);
    assert.equal(value.content, '<script>not executed</script>');
    assert.throws(() => parseConversationExport({ content: {} }));
});
test('all seven locale catalogs have identical complete keys and placeholders', () => {
    const reference = chatConversationMessages.en;
    assert.equal(Object.keys(chatConversationMessages).length, 7);
    for (const messages of Object.values(chatConversationMessages)) {
        assert.deepEqual(Object.keys(messages).sort(), Object.keys(reference).sort());
        for (const [key, value] of Object.entries(messages)) { assert.ok(value.trim()); assert.deepEqual(value.match(/\{[^}]+\}/g) ?? [], reference[key].match(/\{[^}]+\}/g) ?? []); }
    }
});
