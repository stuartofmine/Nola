import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'vite';

test('composer reasoning selector exposes every supported v2 effort in every locale and normalizes inputs', async () => {
    const server = await createServer({ server: { middlewareMode: true, ws: false }, appType: 'custom' });
    try {
        const { default: Chat } = await server.ssrLoadModule('/src/pages/Chat.svelte');
        const { normalizeV2ReasoningEffort } = await server.ssrLoadModule('/src/lib/v2Chat.ts');
        const { render } = await server.ssrLoadModule('svelte/server');
        const { locale, supportedLocales, t } = await server.ssrLoadModule('/src/lib/i18n.ts');
        const supportedEfforts = ['automatic', 'none', 'low', 'medium', 'high'] as const;

        for (const effort of supportedEfforts) {
            assert.equal(normalizeV2ReasoningEffort(effort), effort);
        }
        assert.equal(normalizeV2ReasoningEffort('unknown-level'), 'automatic');
        assert.equal(normalizeV2ReasoningEffort('HIGH'), 'automatic');
        assert.equal(normalizeV2ReasoningEffort(null), 'automatic');

        for (const language of supportedLocales) {
            locale.set(language);
            const html: string = render(Chat).body;
            const reasoningLabel = t(language, 'chatSession.reasoning');
            assert.ok(html.includes(`aria-label="${reasoningLabel}"`), `${language}: reasoning select`);
            for (const effort of supportedEfforts) {
                assert.ok(html.includes(`value="${effort}"`), `${language}: value=${effort}`);
                assert.ok(html.includes(t(language, `chatSession.${effort}`)), `${language}: label=${effort}`);
            }
        }
    } finally {
        await server.close();
    }
});
