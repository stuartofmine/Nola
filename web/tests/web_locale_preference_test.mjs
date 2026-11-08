import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { initializeLocale, locale, localePreference, localeFromCookie, observeLocaleChanges, setLocale } from '../frontend/src/lib/i18n.ts';

function current(store) {
    let value;
    const unsubscribe = store.subscribe(item => { value = item; });
    unsubscribe();
    return value;
}

function environment(body) {
    const names = ['localStorage', 'document', 'navigator', 'location'];
    const saved = names.map(name => Object.getOwnPropertyDescriptor(globalThis, name));
    const values = new Map();
    const document = { cookie: '', documentElement: { lang: '' } };
    const navigator = { languages: ['fr-CA'] };
    const location = { protocol: 'https:' };
    const localStorage = { getItem: key => values.get(key) ?? null, setItem: (key, value) => values.set(key, value) };
    try {
        for (const [name, value] of Object.entries({ localStorage, document, navigator, location })) {
            Object.defineProperty(globalThis, name, { configurable: true, value });
        }
        body({ values, document, navigator });
    } finally {
        names.forEach((name, index) => saved[index]
            ? Object.defineProperty(globalThis, name, saved[index]) : Reflect.deleteProperty(globalThis, name));
        locale.set('en'); localePreference.set('system');
    }
}

test('current UI language choice persists exact preference and cookie without navigation', () => environment(({ values, document }) => {
    document.cookie = 'nola_ui_locale=ja';
    initializeLocale();
    assert.equal(current(locale), 'ja');
    setLocale('de');
    assert.equal(values.get('nola.web.locale'), 'de');
    assert.equal(document.cookie, 'nola_ui_locale=de; Path=/; SameSite=Lax; Max-Age=31536000; Secure');
    initializeLocale();
    assert.equal(current(locale), 'de');
    setLocale('system');
    assert.equal(current(locale), 'fr');
    assert.equal(document.cookie, 'nola_ui_locale=; Path=/; SameSite=Lax; Max-Age=0; Secure');
    setLocale('other');
    assert.equal(current(localePreference), 'system');
}));

test('current UI rejects ambiguous or unsupported cookies', () => {
    for (const cookie of ['nola_ui_locale=ja; nola_ui_locale=de', 'other_nola_ui_locale=ja', 'nola_ui_locale=%3Cscript%3E', 'nola_ui_locale=pt-BR']) {
        assert.equal(localeFromCookie(cookie), undefined);
    }
});

test('switching the current locale never requests a reload that would discard a draft', () => environment(() => {
    const previous = Object.getOwnPropertyDescriptor(globalThis, 'window');
    Object.defineProperty(globalThis, 'window', { configurable: true, value: {
        location: { reload() { assert.fail('locale changes must not reload the draft owner'); },
                    assign() { assert.fail('locale changes must not navigate away'); } },
    } });
    const events = new EventTarget();
    const stop = observeLocaleChanges(events);
    try {
        setLocale('de');
        assert.equal(current(locale), 'de');
        setLocale('fr');
        assert.equal(current(locale), 'fr');
        events.dispatchEvent(new Event('languagechange'));
        assert.equal(current(locale), 'fr', 'Explicit choice survives browser locale events');
    } finally {
        stop();
        if (previous) Object.defineProperty(globalThis, 'window', previous);
        else Reflect.deleteProperty(globalThis, 'window');
    }
}));

test('actual workspace and login controls use the current shared locale registry', () => {
    for (const file of ['components/LanguagePreference.svelte', 'pages/Login.svelte']) {
        const source = readFileSync(new URL(`../frontend/src/${file}`, import.meta.url), 'utf8');
        assert.match(source, /value=\{\$localePreference\}/);
        assert.match(source, /onchange=\{event => setLocale\(event\.currentTarget\.value/);
        assert.match(source, /\{#each supportedLocales as language\}/);
    }
    const settings = readFileSync(new URL('../frontend/src/pages/Settings.svelte', import.meta.url), 'utf8');
    assert.match(settings, /import LanguagePreference from ['"]\.\.\/components\/LanguagePreference\.svelte['"]/);
    assert.match(settings, /<LanguagePreference\s*\/>/);
    const chat = readFileSync(new URL('../frontend/src/pages/Chat.svelte', import.meta.url), 'utf8');
    assert.match(chat, /<WorkspaceListNavigation\s/);
    assert.match(chat, /<WorkspaceResourceNavigation\s/);
    const resources = readFileSync(new URL('../frontend/src/components/WorkspaceResourceNavigation.svelte', import.meta.url), 'utf8');
    assert.match(resources, /\['\/activity',\s*'chatInspector\.list\.activity'\]/);
    assert.match(chat, /openManagementTab\('\/settings\?section=general'\)/);
});
