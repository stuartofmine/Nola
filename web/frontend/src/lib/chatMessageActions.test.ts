import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'vite';
import { messageCopyContent, messageMutationId } from './chatMessageActions.ts';
import type { TimelineMessage } from './chatTimeline.ts';

function message(markdown: string): TimelineMessage {
    return {id: 'message', role: 'assistant', state: 'completed', canonical: null, parts: [
        {source: 'canonical', id: 'text', value: {kind: 'text', part_id: 'text', revision: 1, markdown}},
        {source: 'canonical', id: 'reasoning', value: {kind: 'reasoning', part_id: 'reasoning', revision: 1, summary: 'PRIVATE REASONING'}},
        {source: 'canonical', id: 'tool', value: {kind: 'tool', execution_source: {kind: 'model'}, part_id: 'tool', revision: 1, output: 'TOOL OUTPUT'}},
    ]};
}
test('message copy contains only accepted prose, with distinct plain and markdown formats', () => {
    const row = message('# Title\n\n**Answer** [link](https://example.org)\n\n```sh\necho hi\n```');
    assert.equal(messageCopyContent(row, 'plain'), 'Title\n\nAnswer link\n\necho hi');
    assert.equal(messageCopyContent(row, 'markdown'), row.parts[0].value.markdown);
    for (const format of ['plain', 'markdown'] as const) {
        assert.ok(!messageCopyContent(row, format).includes('PRIVATE'));
        assert.ok(!messageCopyContent(row, format).includes('TOOL OUTPUT'));
    }
    row.parts[0].value.review = 'pending';
    assert.equal(messageCopyContent(row, 'markdown'), '');
});
test('user copy preserves literal text and strips recognized model attachment envelopes', () => {
    const row = {...message('**literal**'), role: 'user' as const};
    assert.equal(messageCopyContent(row, 'plain'), '**literal**');
    row.parts[0].value.markdown = 'Review\n\n' + JSON.stringify({nola_web_text_attachments_version: 1, documents: [{name: 'file', content: 'PRIVATE FILE', clipped: false}]});
    assert.equal(messageCopyContent(row, 'markdown'), 'Review');
});
test('history mutations require a settled persisted boundary, not optimistic or journal-only messages', () => {
    const row = message('');
    assert.equal(messageMutationId(row), null);
    row.canonical = {message_id: row.id, role: row.role, state: row.state, revision: 1, created_at_ms: 1, parts: []};
    assert.equal(messageMutationId(row), row.id);
    assert.equal(messageMutationId({...row, state: 'running'}), null);
    assert.equal(messageMutationId({...row, state: 'pending'}), null);
    assert.equal(messageMutationId({...row, id: 'foreign'}), null);
});
test('message toolbar renders both formats, disables incomplete assistant output, and has translated labels', async () => {
    const server = await createServer({server: {middlewareMode: true, ws: false}, appType: 'custom'});
    try {
        const {default: Component} = await server.ssrLoadModule('/src/components/ChatMessageActions.svelte');
        const {render} = await server.ssrLoadModule('svelte/server');
        const {locale, supportedLocales, t} = await server.ssrLoadModule('/src/lib/i18n.ts');
        for (const language of supportedLocales) {
            locale.set(language);
            const html = render(Component, {props: {message: message('Answer'), ownerKey: 'fixture'}}).body;
            assert.ok(html.includes(t(language, 'chatPage.copyMarkdown')));
            assert.ok(html.includes(t(language, 'shell.markdown.copy')));
            assert.equal((html.match(/<button/g) ?? []).length, 2);
            assert.ok(!html.includes('disabled'));
            const live = render(Component, {props: {message: {...message('Partial'), state: 'running'}, ownerKey: 'fixture'}}).body;
            assert.equal((live.match(/disabled/g) ?? []).length, 2);
            const persisted = message('');
            persisted.canonical = {message_id: persisted.id, role: persisted.role, state: persisted.state, revision: 1, created_at_ms: 1, parts: []};
            const mutations = render(Component, {props: {message: persisted, ownerKey: 'fixture', busy: true, onoperation: () => {}}}).body;
            assert.ok(mutations.includes(t(language, 'chatConversations.branchHere')));
            assert.ok(mutations.includes(t(language, 'chatConversations.truncateHere')));
            assert.equal((mutations.match(/disabled/g) ?? []).length, 2);
        }
    } finally { await server.close(); }
});
