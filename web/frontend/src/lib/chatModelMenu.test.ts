import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { compile } from 'svelte/compiler';
import { render } from 'svelte/server';
import { locale, supportedLocales, t } from './i18n.ts';

const source = readFileSync(new URL('../components/ChatModelMenu.svelte', import.meta.url), 'utf8');
const code = compile(source, { generate: 'server', filename: 'ChatModelMenu.svelte' }).js.code
    .replaceAll("'svelte'", JSON.stringify(import.meta.resolve('svelte')))
    .replaceAll("'svelte/internal/server'", JSON.stringify(import.meta.resolve('svelte/internal/server')))
    .replaceAll("'../lib/i18n'", JSON.stringify(new URL('./i18n.ts', import.meta.url).href));
const { default: Menu } = await import(`data:text/javascript;base64,${Buffer.from(code).toString('base64')}`);
test('header model menu displays selected names, groups and management in every locale', () => {
    for (const language of supportedLocales) {
        locale.set(language);
        const html = render(Menu, { props: {
            selection: { choice: 'cloud:one', disabled: false, errorKey: '', options: [
                { value: 'cloud:one', name: 'Cloud Alpha', detail: 'Host A', local: false },
                { value: 'local:two', name: 'Local Beta', detail: '', local: true },
            ] }, onselect() {}, onsettings() {}, onreload() {},
        } }).body;
        assert.ok(html.includes('Cloud Alpha'));
        assert.ok(html.includes('Local Beta'));
        assert.ok(html.includes('Host A'));
        assert.ok(html.includes('aria-pressed="true"'));
        assert.ok(html.includes(t(language, 'models.local')));
        assert.ok(html.includes('href="/models"'));
    }
});
test('unavailable catalog preserves management and disables selection', () => {
    const html = render(Menu, { props: { selection: { choice: 'cloud:missing', disabled: true, errorKey: 'chatSession.catalogFailed', options: [] }, onselect() {}, onsettings() {}, onreload() {} } }).body;
    assert.ok(html.includes('disabled'));
    assert.ok(html.includes('role="status"'));
    assert.ok(html.includes('href="/models"'));
});
