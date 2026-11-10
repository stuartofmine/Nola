import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { compile } from 'svelte/compiler';
import { render } from 'svelte/server';
import { locale, supportedLocales, t } from './i18n.ts';
import { emptyContentTabs, openPdfContentTab, contentTabDomId } from './conversationContentTabs.ts';

const source = readFileSync(new URL('../components/ConversationContentTabs.svelte', import.meta.url), 'utf8');
const code = compile(source, {generate: 'server', filename: 'ConversationContentTabs.svelte'}).js.code
    .replaceAll("'svelte'", JSON.stringify(import.meta.resolve('svelte')))
    .replaceAll("'svelte/internal/server'", JSON.stringify(import.meta.resolve('svelte/internal/server')))
    .replaceAll("'../lib/i18n'", JSON.stringify(new URL('./i18n.ts', import.meta.url).href))
    .replaceAll("'../lib/conversationContentTabs'", JSON.stringify(new URL('./conversationContentTabs.ts', import.meta.url).href));
const {default: Tabs} = await import(`data:text/javascript;base64,${Buffer.from(code).toString('base64')}`);
test('production tab strip exposes one selected roving tab, stable panel bindings and translated close action', () => {
    const state = openPdfContentTab(emptyContentTabs(), 'owner', {resourceId: 'pdf', conversationId: 'owner',
        displayName: '<script>private.pdf', mimeType: 'application/pdf', byteCount: 12, sha256: 'a'.repeat(64)});
    const ownerKey = JSON.stringify(['host', 'owner']);
    for (const language of supportedLocales) {
        locale.set(language);
        const html = render(Tabs, {props: {state, ownerKey, title: 'Conversation', onselect() {}, onclose() {}}}).body;
        assert.equal((html.match(/role="tab"/g) ?? []).length, 2);
        assert.equal((html.match(/aria-selected="true"/g) ?? []).length, 1);
        assert.equal((html.match(/tabindex="0"/g) ?? []).length, 1);
        assert.ok(html.includes(`aria-controls="${contentTabDomId('panel', ownerKey, 'conversation')}"`));
        assert.ok(!html.includes(contentTabDomId('panel', JSON.stringify(['other-host', 'owner']), 'conversation')));
        assert.ok(html.includes(t(language, 'chatInspector.contentTabs')));
        assert.ok(html.includes(t(language, 'chatInspector.closeTab')));
        assert.equal((html.match(/class="close(?: [^"]*)?"/g) ?? []).length, 1);
        assert.match(html, /&lt;script(?:>|&gt;)private\.pdf/);
        assert.ok(!html.includes('<script>private.pdf'));
    }
});
