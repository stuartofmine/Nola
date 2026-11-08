import { test } from 'node:test';
import assert from 'node:assert/strict';
import { conversationHref, filteredConversations } from './conversations.ts';

test('conversation links preserve opaque identity and cannot become external navigation', () => {
    const id = '../other?x=1&conversation=wrong#part';
    const link = new URL(conversationHref(id), 'https://nola.invalid');
    assert.equal(link.pathname, '/chat');
    assert.equal(link.searchParams.get('conversation'), id);
    assert.equal(link.hash, '');
    assert.equal(link.origin, 'https://nola.invalid');
});

test('conversation search handles multilingual titles and does not mutate server order', () => {
    const input = [
        { conversationId: 'b', title: '论文 Research', updatedAt: '2026-09-22T12:00:00+08:00' },
        { conversationId: 'a', title: '模型配置', updatedAt: '2026-09-22T08:00:00Z' },
    ];
    assert.deepEqual(filteredConversations(input, '').map(value => value.conversationId), ['a', 'b']);
    assert.deepEqual(filteredConversations(input, ' RESEARCH ').map(value => value.conversationId), ['b']);
    assert.deepEqual(filteredConversations(input, '模型').map(value => value.conversationId), ['a']);
    assert.deepEqual(input.map(value => value.conversationId), ['b', 'a']);
});
