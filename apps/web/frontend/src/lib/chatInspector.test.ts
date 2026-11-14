import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'vite';
import { supportedLocales, t } from './i18n.ts';

test('all inspector actions and empty plan states have seven-language labels', () => {
    for (const language of supportedLocales) {
        for (const action of ['progress', 'resources', 'settings', 'close', 'requirements', 'noPlan']) {
            const key = `chatInspector.${action}`;
            assert.notEqual(t(language, key), key);
            assert.ok(t(language, key).trim());
        }
    }
});


test('overview keeps only current status and the action needed to unblock a conversation', async () => {
    const server = await createServer({server: {middlewareMode: true, ws: false}, appType: 'custom'});
    try {
        const {default: Component} = await server.ssrLoadModule('/src/components/ChatDecisionOverview.svelte');
        const {render} = await server.ssrLoadModule('svelte/server');
        const {locale, supportedLocales, t} = await server.ssrLoadModule('/src/lib/i18n.ts');
        for (const language of supportedLocales) {
            locale.set(language);
            for (const needsResponse of [false, true]) {
                const html = render(Component, {props: {working: true, needsResponse, onconversation() {}}}).body;
                assert.equal((html.match(/<button /g) ?? []).length, needsResponse ? 1 : 0);
                assert.ok(!html.includes('<nav'));
                assert.ok(!html.includes('class="deliverables"'));
                if (needsResponse) assert.ok(html.includes(t(language, 'chatInspector.handleResponse')));
            }
        }
    } finally { await server.close(); }
});
