import test from 'node:test';
import assert from 'node:assert/strict';
import { conversationPdfUrl, conversationResourcesUrl, parseConversationPdfResource, parseConversationPdfResources, pdfFileBase64, validatePdfFile } from './chatResources.ts';
import { chatResourceMessages } from './locales/chatResources.ts';

const conversationId = 'conv-1';
const resourceId = 'f9779bce-2fa7-4d26-b358-0586e3cb42be';
const descriptor = { resourceId, conversationId, displayName: 'paper.pdf', mimeType: 'application/pdf',
    byteCount: 9, sha256: 'a'.repeat(64), previewUrl: conversationPdfUrl(conversationId, resourceId) };

test('PDF preview identity is conversation-owned and server URLs are never trusted', () => {
    assert.equal(conversationResourcesUrl('space /?'), '/api/chat/conversations/space%20%2F%3F/resources');
    assert.deepEqual(parseConversationPdfResources({ conversationId, resources: [descriptor] }, conversationId), [
        { resourceId, conversationId, displayName: 'paper.pdf', mimeType: 'application/pdf', byteCount: 9, sha256: 'a'.repeat(64) },
    ]);
    for (const change of [{ conversationId: 'another' }, { previewUrl: 'https://example.com/steal' }, { mimeType: 'text/html' }, { resourceId: '../escape' }]) {
        assert.throws(() => parseConversationPdfResource({ ...descriptor, ...change }, conversationId));
    }
    assert.throws(() => parseConversationPdfResources({ conversationId, resources: [descriptor, descriptor] }, conversationId));
});

test('PDF upload validates metadata and encodes bytes without a stack-sized spread', async () => {
    const bytes = Uint8Array.from([37, 80, 68, 70, 45, 49, 46, 55, 10]);
    const file = new File([bytes], 'paper.pdf', { type: 'application/pdf' });
    assert.equal(await pdfFileBase64(file), Buffer.from(bytes).toString('base64'));
    assert.doesNotThrow(() => validatePdfFile({ name: 'paper.PDF', type: '', size: 9 }));
    assert.throws(() => validatePdfFile({ name: 'paper.pdf', type: 'application/pdf', size: 8 * 1024 * 1024 + 1 }));
    assert.throws(() => validatePdfFile({ name: 'fake.pdf', type: 'text/html', size: 9 }));
});

test('PDF preview actions and failures are translated in every supported language', () => {
    for (const catalog of Object.values(chatResourceMessages)) {
        for (const key of ['upload', 'previewOnly', 'previewFailed', 'invalidFile']) assert.ok(catalog[`chatResource.${key}`]);
    }
});
