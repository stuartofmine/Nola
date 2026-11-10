import test from 'node:test';
import assert from 'node:assert/strict';
import { ConversationCacheReader } from './conversationCacheReader.ts';
import type { ConversationCacheUsage } from './conversationCacheUsage.ts';

function fixture() {
    const calls: { id: string; signal: AbortSignal; resolve: (value: unknown) => void; reject: (error: Error) => void }[] = [];
    const published: { usage: ConversationCacheUsage; failed: boolean }[] = [];
    const reader = new ConversationCacheReader((id, signal) => new Promise((resolve, reject) => {
        calls.push({ id, signal, resolve, reject });
    }), (usage, failed) => published.push({ usage, failed }));
    return { reader, calls, published };
}
const totals = (id: string, read = 40) => ({ conversation_id: id, usage: { conversation_cache_usage: {
    input_tokens: 100, read_tokens: read, write_tokens: null, hit_rate_bps: read * 100,
    is_compaction: false, reported_requests: 2, unreported_requests: 1,
} } });
const settle = async () => { await Promise.resolve(); await Promise.resolve(); };

test('switch aborts and fences old results even if transport ignores cancellation', async () => {
    const { reader, calls, published } = fixture();
    reader.select('a'); reader.select('b');
    assert.equal(calls[0].signal.aborted, true);
    calls[1].resolve(totals('b', 70)); await settle();
    calls[0].resolve(totals('a')); await settle();
    assert.equal(published.at(-1)?.usage?.read_tokens, 70);
    assert.equal(published.length, 3);
});

test('reselecting the same conversation creates a new generation', async () => {
    const { reader, calls, published } = fixture();
    reader.select('a'); reader.select('a');
    calls[0].reject(new Error('stale failure')); await settle();
    assert.equal(published.length, 2);
    calls[1].resolve(totals('a')); await settle();
    assert.equal(published.at(-1)?.failed, false);
});

test('many refresh boundaries coalesce into one follow-up and preserve durable coverage', async () => {
    const { reader, calls, published } = fixture();
    reader.select('a');
    for (let i = 0; i < 100; i++) reader.refresh();
    assert.equal(calls.length, 1);
    calls[0].resolve(totals('a')); await settle();
    assert.equal(calls.length, 2);
    calls[1].resolve(totals('a', 80)); await settle();
    assert.equal(calls.length, 2);
    assert.equal(published.at(-1)?.usage?.reported_requests, 2);
    assert.equal(published.at(-1)?.usage?.unreported_requests, 1);
    assert.equal(published.at(-1)?.usage?.hit_rate_bps, 8000);
});

test('failed or wrong-owner refresh clears stale value without manufacturing zero', async () => {
    const { reader, calls, published } = fixture();
    reader.select('a'); calls[0].resolve(totals('a')); await settle();
    reader.refresh(); calls[1].reject(new Error('offline')); await settle();
    assert.deepEqual(published.at(-1), { usage: null, failed: true });
    reader.refresh(); calls[2].resolve(totals('b')); await settle();
    assert.deepEqual(published.at(-1), { usage: null, failed: true });
    reader.refresh(); calls[3].resolve({ conversation_id: 'a', usage: {} }); await settle();
    assert.deepEqual(published.at(-1), { usage: null, failed: false });
});

test('dispose cancels current and queued reads and cannot publish late results', async () => {
    const { reader, calls, published } = fixture();
    reader.select('a'); reader.refresh(); reader.dispose();
    assert.equal(calls[0].signal.aborted, true);
    calls[0].resolve(totals('a')); await settle();
    reader.refresh();
    assert.equal(calls.length, 1);
    assert.equal(published.length, 1);
});
