import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'vite';
import { chatReference, timelineReference, toolArtifacts } from './chatReferences.ts';
import { journalPart } from './journalTimeline.fixture.ts';

const identity = {source_store_epoch: 'epoch', conversation_id: 'conversation', run_id: 'run', assistant_message_id: 'assistant'};
const value = {kind: 'citation', part_id: 'citation', revision: 1, title: 'Evidence', uri: 'https://example.org/report',
    source_id: 'source', location: {page: 0, line_start: 12, line_end: 15, section: 'Methods'}, range_start: 0, range_end: 80};

test('live and restored references preserve identical location and identity, including zero offsets', () => {
    const restored = timelineReference({source: 'canonical', id: value.part_id, value});
    const live = timelineReference(journalPart(identity, value));
    assert.deepEqual(live, restored);
    assert.equal(live?.page, '0'); assert.equal(live?.rangeStart, '0');
    assert.equal(live?.sourceId, 'source'); assert.equal(live?.section, 'Methods');
    assert.equal(chatReference({...value, range_start: '9007199254740993'})?.rangeStart, '9007199254740993');
    assert.equal(chatReference({...value, page: NaN, location: null})?.page, '');
});

test('reference links never create local-file or privileged browser authority', () => {
    for (const uri of ['file:///etc/passwd', '/etc/passwd', 'javascript:alert(1)', 'data:text/html,x', 'https://user:secret@example.org/', '//example.org', 'mailto:test@example.org']) {
        assert.equal(chatReference({...value, uri})?.href, undefined, uri);
    }
    assert.equal(chatReference({...value, kind: 'tool'}), null);
    assert.equal(toolArtifacts([null, 5, {name: 'report', uri: 'file:///report.pdf'}]).length, 1);
    assert.equal(toolArtifacts([{name: 'report', uri: 'file:///report.pdf'}])[0].href, undefined);
});

test('reference UI renders canonical and journal facts in every locale without activating local paths', async () => {
    const server = await createServer({server: {middlewareMode: true, ws: false}, appType: 'custom'});
    try {
        const {default: Component} = await server.ssrLoadModule('/src/components/ChatReferenceCard.svelte');
        const {render} = await server.ssrLoadModule('svelte/server');
        const {locale, supportedLocales, t} = await server.ssrLoadModule('/src/lib/i18n.ts');
        for (const language of supportedLocales) {
            locale.set(language);
            const html = render(Component, {props: {reference: chatReference(value)}}).body;
            for (const label of ['Evidence', 'Methods', 'source', '12–15', '[0, 80)', t(language, 'chatPage.referencePage')]) assert.ok(html.includes(label), label);
            assert.ok(html.includes('href="https://example.org/report"'));
            const local = render(Component, {props: {reference: chatReference({...value, uri: 'file:///report.pdf', title: '<script>unsafe</script>'})}}).body;
            assert.ok(!local.includes('href=')); assert.ok(!local.includes('<script>unsafe'));
            assert.ok(local.includes(t(language, 'chatPage.referenceUnavailable')));
        }
    } finally { await server.close(); }
});
