import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'vite';

test('composer reasoning menu exposes every supported effort with a confirmed selection', async () => {
    const server = await createServer({ server: { middlewareMode: true, ws: false }, appType: 'custom' });
    try {
        const { default: Menu } = await server.ssrLoadModule('/src/components/ChatReasoningMenu.svelte');
        const { render } = await server.ssrLoadModule('svelte/server');
        const { locale, supportedLocales, t } = await server.ssrLoadModule('/src/lib/i18n.ts');
        const state = { preference: 'medium', options: ['none', 'low', 'medium', 'high', 'xhigh'], ready: true, busy: false, errorKey: '' };
        for (const language of supportedLocales) {
            locale.set(language);
            const html = render(Menu, { props: { state, onselect: () => {} } }).body;
            for (const effort of state.options) assert.ok(html.includes(t(language, `chatSession.${effort}`)));
            assert.equal((html.match(/<button /g) ?? []).length, state.options.length + 2);
            assert.equal((html.match(/aria-pressed="true"/g) ?? []).length, 1);
            const locked = render(Menu, { props: { state: { ...state, busy: true }, onselect: () => {} } }).body;
            assert.equal((locked.match(/<button[^>]* disabled/g) ?? []).length, state.options.length + 2);
            assert.ok(!render(Menu, { props: { state: { ...state, options: [] }, onselect: () => {} } }).body.includes('<details'));
        }
    } finally { await server.close(); }
});
