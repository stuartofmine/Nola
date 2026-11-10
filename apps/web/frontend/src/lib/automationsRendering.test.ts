import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createServer } from 'vite';
import { supportedLocales, t } from './i18n.ts';
import { automationMessages } from './locales/automations.ts';
import { automationFormats, automationCreateRequest, newAutomationDraft, parseAutomations, automationCapabilities } from './automations.ts';

const escapeText = (value: string) => value.replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;');

test('actual automation Svelte page renders seven-language controls and unknown capabilities fail closed before mount', async () => {
    const server = await createServer({server: {middlewareMode: true, ws: false}, appType: 'custom'});
    try {
    const {default: Automations} = await server.ssrLoadModule('/src/pages/Automations.svelte');
    const {render} = await server.ssrLoadModule('svelte/server');
    const {locale: renderedLocale} = await server.ssrLoadModule('/src/lib/i18n.ts');
    for (const language of supportedLocales) {
        renderedLocale.set(language);
        const html = render(Automations).body.replace(/<!--.*?-->/gs, '');
        assert.equal(html.match(/<h1(?:\s[^>]*)?>(.*?)<\/h1>/s)?.[1], escapeText(t(language, 'automation.title')), language);
        for (const key of ['create', 'refresh', 'search', 'filter', 'pending']) {
            assert.ok(html.includes(escapeText(t(language, `automation.${key}`))), `${language}: ${key}`);
        }
        assert.ok(!html.includes(escapeText(t(language, 'automation.executionUnavailable'))), `${language}: loading must not claim unavailable`);
        assert.ok(!html.includes('automation.hostScheduleHint'));
        assert.ok(!html.includes('automation.invalidResponse'));
        assert.ok(!html.includes(escapeText(t(language, 'automation.scheduledAvailable'))));
    }
    } finally { await server.close(); }
});

test('explicit host capability snapshots distinguish manual-only and scheduled support', () => {
    assert.deepEqual(automationCapabilities(null), { manualRun: false, scheduledDispatch: false, promptAutomations: false });
    assert.deepEqual(automationCapabilities({ executionCapabilities: { manualRun: true, scheduledDispatch: false, promptAutomations: false } }), { manualRun: true, scheduledDispatch: false, promptAutomations: false });
    assert.deepEqual(automationCapabilities({ executionCapabilities: { manualRun: true, scheduledDispatch: true, promptAutomations: true } }), { manualRun: true, scheduledDispatch: true, promptAutomations: true });
    assert.deepEqual(automationCapabilities({ executionCapabilities: { manualRun: 'true', scheduledDispatch: 1 } }), { manualRun: false, scheduledDispatch: false, promptAutomations: false });
    for (const language of supportedLocales) {
        for (const key of ['executionUnavailable', 'hostScheduleHint', 'scheduledAvailable']) assert.notEqual(t(language, `automation.${key}`), `automation.${key}`);
    }
});

test('page and helper error keys and dynamic approval/type labels exist in every automation locale', () => {
    const source = readFileSync(new URL('../pages/Automations.svelte', import.meta.url), 'utf8');
    const literalKeys = [...source.matchAll(/['"](automation\.[A-Za-z0-9_.]+)['"]/g)].map(match => match[1]);
    const dynamicKeys = [
        ...automationFormats.map(format => `automation.format.${format}`), 'automation.format.reminder',
        ...['all', 'enabled', 'disabled', 'pending', 'daily', 'weekly', 'runConfirmTitle', 'runConfirmBody', 'approveTitle', 'approveBody', 'deleteTitle', 'deleteBody'].map(key => `automation.${key}`),
    ];
    const errorKeys: string[] = [];
    for (const action of [() => parseAutomations(null), () => automationCreateRequest(newAutomationDraft())]) {
        try { action(); assert.fail('fixture must fail validation'); }
        catch (error) { assert.ok(error instanceof Error); errorKeys.push(error.message); }
    }
    for (const language of supportedLocales) {
        for (const key of [...literalKeys, ...dynamicKeys, ...errorKeys]) {
            assert.ok(automationMessages[language][key]?.trim(), `${language}: ${key}`);
            assert.notEqual(t(language, key), key, `registered ${language}: ${key}`);
        }
    }
});
