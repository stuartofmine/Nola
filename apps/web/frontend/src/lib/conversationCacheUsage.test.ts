import test from 'node:test';
import assert from 'node:assert/strict';
import { parseConversationCacheUsage } from './conversationCacheUsage.ts';

test('conversation cache consumes durable totals, not the latest request', () => {
    const totals = { input_tokens: 2000, read_tokens: 900, write_tokens: null,
        hit_rate_bps: 4500, is_compaction: false, reported_requests: 3, unreported_requests: 2 };
    const response = { conversation_id: 'a', usage: { conversation_cache_usage: totals,
        cache_usage: { input_tokens: 100, read_tokens: 100, hit_rate_bps: 10000 } } };
    assert.deepEqual(parseConversationCacheUsage(response, 'a'), totals);
    assert.throws(() => parseConversationCacheUsage(response, 'b'), /owner/);
});

test('unreported requests remain unknown, not zero percent', () => {
    assert.equal(parseConversationCacheUsage({ conversation_id: 'a', usage: {} }, 'a'), null);
    const totals = { input_tokens: null, read_tokens: null, write_tokens: null,
        hit_rate_bps: null, is_compaction: false, reported_requests: 0, unreported_requests: 2 };
    assert.equal(parseConversationCacheUsage({ conversation_id: 'a', usage: { conversation_cache_usage: totals } }, 'a')?.hit_rate_bps, null);
    assert.throws(() => parseConversationCacheUsage({ conversation_id: 'a', usage: {
        conversation_cache_usage: { ...totals, hit_rate_bps: 0 } } }, 'a'), /rate/);
});

test('known zero counters and incomplete cumulative coverage remain distinct', () => {
    const totals = { input_tokens: 0, read_tokens: 0, write_tokens: null,
        hit_rate_bps: null, is_compaction: false, reported_requests: 1, unreported_requests: 3 };
    const parsed = parseConversationCacheUsage({ conversation_id: 'a', usage: { conversation_cache_usage: totals } }, 'a');
    assert.deepEqual(parsed, totals);
    assert.equal(parsed?.input_tokens, 0);
    assert.equal(parsed?.write_tokens, null);
    assert.equal(parsed?.unreported_requests, 3);
    assert.equal(parsed?.hit_rate_bps, null); // No denominator, not a fabricated 0% hit rate.
});
