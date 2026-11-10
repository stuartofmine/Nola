import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'vite';

test('empty API catalog still offers Svelte endpoint and model creation in every locale', async () => {
    const server = await createServer({ server: { middlewareMode: true, ws: false }, appType: 'custom' });
    try {
        const { default: RemoteProfiles } = await server.ssrLoadModule('/src/pages/RemoteProfiles.svelte');
        const { locale, supportedLocales, t } = await server.ssrLoadModule('/src/lib/i18n.ts');
        const { render } = await server.ssrLoadModule('svelte/server');
        for (const language of supportedLocales) {
            locale.set(language);
            const html: string = render(RemoteProfiles, { props: { profiles: [], reload: async () => {}, onSelectionSaved: () => {} } }).body;
            assert.ok(html.includes(t(language, 'remote.add_endpoint_model')));
            assert.ok(html.includes('<dialog'));
        }
    } finally { await server.close(); }
});

test('configured Svelte API catalog exposes ordered model and endpoint controls', async () => {
    const server = await createServer({ server: { middlewareMode: true, ws: false }, appType: 'custom' });
    try {
        const { default: RemoteProfiles } = await server.ssrLoadModule('/src/pages/RemoteProfiles.svelte');
        const { locale, t } = await server.ssrLoadModule('/src/lib/i18n.ts');
        const { render } = await server.ssrLoadModule('svelte/server');
        locale.set('zh-Hans');
        const endpoint = { id: 'endpoint-a', name: 'A', baseURL: 'https://example.test/v1', wireProtocol: 'responses', enabled: true, order: 0 };
        const model = (id: string, order: number) => ({ id, provider: 'openai-compatible', displayName: id, modelName: id,
            revision: 1, enabled: true, selected: order === 0, priority: 50, order, endpoint,
            capabilities: ['chat'], reasoning: null, parameters: { maxOutputTokens: 16384 } });
        const html: string = render(RemoteProfiles, { props: { profiles: [model('model-a', 0), model('model-b', 1)],
            reload: async () => {}, onSelectionSaved: () => {} } }).body;
        assert.ok(html.includes(t('zh-Hans', 'remote.routing_policy')));
        assert.ok(html.includes(t('zh-Hans', 'remote.add_model')));
        assert.ok(!html.includes(t('zh-Hans', 'models.discard')), 'A clean catalog has no edits to discard');
        assert.ok(!html.includes(t('zh-Hans', 'remote.saved')), 'Opening a catalog must not claim it was saved');
        assert.ok(html.includes(t('zh-Hans', 'remote.models')));
        assert.ok(html.includes(t('zh-Hans', 'remote.more_actions')));
        assert.match(html, /class="compact-actions\b/);
        assert.ok(html.includes(t('zh-Hans', 'remote.discover')));
        assert.match(html, /aria-label="启用端点"/);
        assert.match(html, /aria-label="启用模型: model-a"/);
        assert.match(html, /aria-label="启用模型: model-b"/);
        assert.ok(html.includes(t('zh-Hans', 'remote.delete_model')));
        assert.ok(html.includes(t('zh-Hans', 'remote.delete_endpoint')));
        assert.match(html, /aria-label="上移"/);
        assert.match(html, /aria-label="下移"/);
    } finally { await server.close(); }
});

test('unknown models remain visible and distinct from text-only models in every locale', async () => {
    const server = await createServer({ server: { middlewareMode: true, ws: false }, appType: 'custom' });
    try {
        const { default: RemoteProfiles } = await server.ssrLoadModule('/src/pages/RemoteProfiles.svelte');
        const { locale, supportedLocales, t } = await server.ssrLoadModule('/src/lib/i18n.ts');
        const { render } = await server.ssrLoadModule('svelte/server');
        const endpoint = { id: 'endpoint', name: 'Fixture', baseURL: 'https://example.test/v1', wireProtocol: 'responses', enabled: true, order: 0 };
        const unknown = { id: 'unknown', provider: 'openai-compatible', displayName: 'Unknown fixture', modelName: 'unknown-model',
            revision: 1, enabled: true, selected: true, priority: 50, order: 0, endpoint,
            capabilities: ['chat'], executionCapabilities: null, reasoning: null, parameters: { maxOutputTokens: 16384 } };
        const textOnly = { ...unknown, id: 'text-only', displayName: 'Text-only fixture', modelName: 'text-only-model', order: 1,
            executionCapabilities: { toolCalls: false, imageInput: false, reasoning: false, samplingParameters: false } };
        for (const language of supportedLocales) {
            locale.set(language);
            const html: string = render(RemoteProfiles, { props: { profiles: [unknown, textOnly],
                reload: async () => {}, onSelectionSaved: () => {} } }).body;
            assert.ok(html.includes('Unknown fixture'));
            assert.ok(html.includes('Text-only fixture'));
            for (const key of ['remote.execution_confirm_required', 'remote.execution_text_only', 'remote.execution_confirm_add']) {
                const translated = t(language, key);
                assert.notEqual(translated, key);
                assert.ok(html.includes(translated));
            }
            assert.equal(unknown.executionCapabilities, null);
        }
    } finally { await server.close(); }
});
