import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createServer } from 'vite';
import { supportedLocales, t } from './i18n.ts';

test('composer model selector and unconfigured-model recovery link stay wired in v2 Chat', () => {
    const source = readFileSync(new URL('../pages/Chat.svelte', import.meta.url), 'utf8');
    assert.match(source, /aria-label=\{t\(\$locale,\s*'chatSession\.model'\)\}/);
    assert.match(source, /onchange=\{e => void handleModelChange\(e\.currentTarget\.value\)\}/);
    assert.match(source, /\{#if needsModelConfig\}[^]*?href="\/models"[^]*?chatSession\.configureModel/);
    assert.match(source, /\{#if modelChoices\.length\}[^]*?<select/);
    for (const language of supportedLocales) {
        for (const key of ['chatSession.model', 'chatSession.configureModel', 'models.local']) {
            assert.notEqual(t(language, key), key);
            assert.ok(t(language, key).trim());
        }
    }
});

test('v2 Chat toolbar renders localized permission and reasoning selectors without dead model choices', async () => {
    const server = await createServer({ server: { middlewareMode: true, ws: false }, appType: 'custom' });
    try {
        const { default: Chat } = await server.ssrLoadModule('/src/pages/Chat.svelte');
        const { render } = await server.ssrLoadModule('svelte/server');
        const { locale, supportedLocales: languages, t: translate } = await server.ssrLoadModule('/src/lib/i18n.ts');
        for (const language of languages) {
            locale.set(language);
            const html: string = render(Chat).body;
            // Empty model catalog does not render an empty dead model <select>.
            assert.ok(!html.includes(`aria-label="${translate(language, 'chatSession.model')}"`));
            assert.ok(html.includes(`aria-label="${translate(language, 'chatSession.permission')}"`));
            assert.ok(html.includes(`aria-label="${translate(language, 'chatSession.reasoning')}"`));
            assert.ok(!html.includes('undefined'));
        }
    } finally {
        await server.close();
    }
});
