import { test } from 'node:test';
import assert from 'node:assert/strict';
import { ApiError } from './api.ts';
import { chatEventStream, createChatEventDecoder, ChatTransportError } from './chatTransport.ts';
import { locale } from './i18n.ts';

test('SSE framing preserves all CR/LF chunk boundaries, comments and multiline Unicode JSON', () => {
    const source = '\uFEFF: heartbeat\r\nevent: update\r\nid: ignored\r\ndata: {"type":"assistant_journal_committed_v1",\r\ndata: "text":"你好 👋"}\r\n\r\ndata: [DONE]\r\n\r\n';
    for (let split = 0; split <= source.length; split++) {
        const parser = createChatEventDecoder();
        const values = [...parser.push(source.slice(0, split)), ...parser.push(source.slice(split))];
        parser.finish();
        assert.deepEqual(values, [{ type: 'assistant_journal_committed_v1', text: '你好 👋' }], `boundary ${split}`);
    }
    for (const newline of ['\n', '\r', '\r\n']) {
        const parser = createChatEventDecoder();
        assert.deepEqual([...`data:{"x":1}${newline}${newline}`].flatMap(character => parser.push(character)), [{ x: 1 }]);
        parser.finish();
    }
});

test('malformed or truncated frames cannot silently disappear or become durable events', () => {
    for (const payload of ['not-json', 'null', '[]', '"private secret"', '123']) {
        const parser = createChatEventDecoder();
        assert.throws(() => parser.push(`data:${payload}\n\n`), (error: unknown) => error instanceof ChatTransportError && error.code === 'invalid_frame' && !error.message.includes(payload));
        assert.throws(() => parser.push('data:{}\n\n'));
    }
    for (const trailing of ['data:{"type":"conversation_projection_committed_v1"}', 'data:{}\n', 'data']) {
        const parser = createChatEventDecoder(); parser.push(trailing);
        assert.throws(() => parser.finish(), (error: unknown) => error instanceof ChatTransportError && error.code === 'incomplete_frame');
    }
    const parser = createChatEventDecoder();
    assert.deepEqual(parser.push('data:[DONE]\n\n'), []);
    parser.finish();
});

test('frame limits bound partial data and reset only at an actual frame boundary', () => {
    const parser = createChatEventDecoder(32);
    for (let index = 0; index < 100; index++) assert.deepEqual(parser.push('data:{}\n\n'), [{}]);
    assert.throws(() => parser.push(`data:${'x'.repeat(40)}`), (error: unknown) => error instanceof ChatTransportError && error.code === 'frame_too_large');
    for (const limit of [0, -1, NaN, Infinity, 1.5]) assert.throws(() => createChatEventDecoder(limit));
});

test('stream uses same-origin locale/CSRF and handles every byte boundary without retrying POST', async t => {
    Object.defineProperty(globalThis, 'document', { configurable: true, value: { cookie: 'nola_web_session=private; nola_web_csrf=csrf' } });
    t.after(() => { Reflect.deleteProperty(globalThis, 'document'); locale.set('en'); });
    locale.set('ja');
    const bytes = new TextEncoder().encode('data:{"type":"assistant_journal_committed_v1","text":"中文 👋"}\r\n\r\ndata:[DONE]\n\n');
    let requests = 0;
    t.mock.method(globalThis, 'fetch', async (path: string, init: RequestInit) => {
        requests++;
        assert.equal(path, '/api/chat/agent/v2/stream');
        assert.equal(init.method, 'POST'); assert.equal(init.body, '{"submission_id":"stable"}');
        assert.equal(init.credentials, 'same-origin'); assert.equal(init.redirect, 'error');
        const headers = new Headers(init.headers);
        assert.equal(headers.get('Accept-Language'), 'ja'); assert.equal(headers.get('X-Nola-CSRF'), 'csrf');
        assert.equal(headers.get('Accept'), 'text/event-stream');
        return new Response(new ReadableStream({ start(controller) { for (const byte of bytes) controller.enqueue(Uint8Array.of(byte)); controller.close(); } }), { headers: { 'Content-Type': 'text/event-stream; charset=utf-8' } });
    });
    const events = [];
    for await (const event of chatEventStream('/api/chat/agent/v2/stream', { method: 'POST', body: '{"submission_id":"stable"}' })) events.push(event);
    assert.deepEqual(events, [{ type: 'assistant_journal_committed_v1', text: '中文 👋' }]);
    assert.equal(requests, 1);
});

test('HTTP/HTML/invalid UTF-8 fail safely, do not leak response bodies and never retry', async t => {
    Object.defineProperty(globalThis, 'document', { configurable: true, value: { cookie: '' } });
    t.after(() => Reflect.deleteProperty(globalThis, 'document'));
    const replies = [new Response('private-token', { status: 403 }), new Response('<html>private-token</html>'), new Response(Uint8Array.of(0xff), { headers: { 'Content-Type': 'text/event-stream' } })];
    let requests = 0;
    t.mock.method(globalThis, 'fetch', async () => { requests++; return replies.shift()!; });
    const consume = async () => { for await (const _event of chatEventStream('/api/chat/agent/v2/stream', { method: 'POST', body: '{}' })) { /* drain */ } };
    await assert.rejects(consume(), (error: unknown) => error instanceof ApiError && error.status === 403 && !error.message.includes('private-token'));
    await assert.rejects(consume(), (error: unknown) => error instanceof ChatTransportError && error.code === 'invalid_content_type');
    await assert.rejects(consume(), (error: unknown) => error instanceof ChatTransportError && error.code === 'invalid_utf8');
    assert.equal(requests, 3);
});

test('breaking consumption cancels the stream and releases its reader', async t => {
    Object.defineProperty(globalThis, 'document', { configurable: true, value: { cookie: '' } });
    t.after(() => Reflect.deleteProperty(globalThis, 'document'));
    let cancelled = false;
    const stream = new ReadableStream<Uint8Array>({ start(controller) { controller.enqueue(new TextEncoder().encode('data:{"x":1}\n\n')); }, cancel() { cancelled = true; } });
    t.mock.method(globalThis, 'fetch', async () => new Response(stream, { headers: { 'Content-Type': 'text/event-stream' } }));
    for await (const _event of chatEventStream('/api/chat/agent/v2/stream', { method: 'POST' })) break;
    assert.equal(cancelled, true); assert.equal(stream.locked, false);
});

test('aborting between events prevents buffered events from leaking to a successor turn', async t => {
    Object.defineProperty(globalThis, 'document', { configurable: true, value: { cookie: '' } });
    t.after(() => Reflect.deleteProperty(globalThis, 'document'));
    const cancellation = new AbortController(); let count = 0;
    t.mock.method(globalThis, 'fetch', async () => new Response('data:{"x":1}\n\ndata:{"x":2}\n\n', { headers: { 'Content-Type': 'text/event-stream' } }));
    await assert.rejects(async () => {
        for await (const _event of chatEventStream('/api/chat/agent/v2/stream', { method: 'POST', signal: cancellation.signal })) {
            count++; cancellation.abort();
        }
    }, { name: 'AbortError' });
    assert.equal(count, 1);
});
