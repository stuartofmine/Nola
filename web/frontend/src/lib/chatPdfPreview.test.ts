import test from 'node:test';
import assert from 'node:assert/strict';
import { loadConversationPdf } from './chatPdfPreview.ts';
import type { ConversationPdfResource } from './chatResources.ts';

test('shared inspector/tab reader keeps same-origin identity, cancellation and PDF response bounds', async context => {
    const previous = Object.getOwnPropertyDescriptor(globalThis, 'document');
    Object.defineProperty(globalThis, 'document', {value: {cookie: ''}, configurable: true});
    context.after(() => { if (previous) Object.defineProperty(globalThis, 'document', previous); else Reflect.deleteProperty(globalThis, 'document'); });
    const resource: ConversationPdfResource = {resourceId: 'f9779bce-2fa7-4d26-b358-0586e3cb42be', conversationId: 'owner /',
        displayName: 'file.pdf', byteCount: 9, mimeType: 'application/pdf', sha256: 'a'.repeat(64)};
    const signal = new AbortController().signal;
    let status = 200, type = 'application/pdf', size = 9;
    context.mock.method(globalThis, 'fetch', async (path: string, options: RequestInit) => {
        assert.equal(path, '/api/chat/conversations/owner%20%2F/resources/f9779bce-2fa7-4d26-b358-0586e3cb42be');
        assert.equal(options.credentials, 'same-origin');
        assert.equal(options.redirect, 'error');
        assert.equal(options.cache, 'no-store');
        assert.equal(options.signal, signal);
        return new Response(new Uint8Array(size), {status, headers: {'Content-Type': type}});
    });
    assert.equal((await loadConversationPdf(resource, signal)).size, 9);
    type = 'text/html'; await assert.rejects(loadConversationPdf(resource, signal));
    type = 'application/pdf'; status = 404; await assert.rejects(loadConversationPdf(resource, signal));
    status = 200; size = 10; await assert.rejects(loadConversationPdf(resource, signal));
});
