import { test } from 'node:test';
import assert from 'node:assert/strict';
import { get } from 'svelte/store';
import { readFileSync, readdirSync } from 'node:fs';
import { initializeLocale, locale, localeFromCookie, localePreference, messageCatalogs, normalizeLocale, observeLocaleChanges, resolveLocale, setLocale, supportedLocales, t } from './i18n.ts';

test('locale selection honors explicit preferences then supported browser languages', () => {
    assert.equal(resolveLocale('ja', ['zh-CN', 'en-US']), 'ja');
    assert.equal(resolveLocale('system', ['xx', 'fr-CA', 'en']), 'fr');
    assert.equal(resolveLocale('invalid', ['de-DE']), 'de');
    assert.equal(resolveLocale(null, ['unknown']), 'en');
    assert.equal(normalizeLocale('ZH_cn'), 'zh-Hans');
    assert.equal(normalizeLocale('pt-BR'), undefined);
});

test('localized interpolation preserves values as literal text and does not recurse', () => {
    assert.equal(t('en', 'models.configureTitle', { name: '<script>{name}</script>' }), '<script>{name}</script> local model configuration');
    assert.equal(t('zh-Hans', 'models.configureTitle', { name: 'A' }), 'A 本地模型配置');
    assert.equal(t('en', 'missing.key'), 'missing.key');
});

test('legacy-page locale cookie accepts only the exact supported enum', () => {
    assert.equal(localeFromCookie('nola_web_session=secret; nola_ui_locale=ja'), 'ja');
    for (const cookie of ['other_nola_ui_locale=ja', 'nola_ui_locale=zh-CN', 'nola_ui_locale=system', 'nola_ui_locale=%3Cscript%3E', 'nola_ui_locale=unknown', 'nola_ui_locale=', 'nola_ui_locale=ja; nola_ui_locale=de']) {
        assert.equal(localeFromCookie(cookie), undefined);
    }
});

test('language preference survives reload and restricted storage still updates document language', () => {
    const names = ['localStorage', 'document', 'navigator', 'location'] as const;
    const descriptors = names.map(name => Object.getOwnPropertyDescriptor(globalThis, name));
    let saved = 'ja';
    const root = { lang: '' };
    const browser = { languages: ['fr-CA'] };
    const events = new EventTarget();
    const stop = observeLocaleChanges(events as Window);
    try {
        Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: { getItem: () => saved, setItem: (_key: string, value: string) => { saved = value; } } });
        Object.defineProperty(globalThis, 'document', { configurable: true, value: { documentElement: root } });
        Object.defineProperty(globalThis, 'navigator', { configurable: true, value: browser });
        Object.defineProperty(globalThis, 'location', { configurable: true, value: { protocol: 'https:' } });
        initializeLocale();
        assert.equal(get(locale), 'ja');
        assert.equal(root.lang, 'ja');
        setLocale('de');
        assert.equal(saved, 'de');
        assert.equal(document.cookie, 'nola_ui_locale=de; Path=/; SameSite=Lax; Max-Age=31536000; Secure');
        initializeLocale();
        assert.equal(get(locale), 'de');
        setLocale('system');
        assert.equal(document.cookie, 'nola_ui_locale=; Path=/; SameSite=Lax; Max-Age=0; Secure');
        assert.equal(get(localePreference), 'system');
        assert.equal(root.lang, 'fr');
        browser.languages = ['ru'];
        events.dispatchEvent(new Event('languagechange'));
        assert.equal(root.lang, 'ru');
        setLocale('de');
        browser.languages = ['ja'];
        events.dispatchEvent(new Event('languagechange'));
        assert.equal(root.lang, 'de', 'An explicit choice must not follow the browser');
        saved = 'es';
        const changed = new Event('storage');
        Object.defineProperty(changed, 'key', { value: 'nola.web.locale' });
        events.dispatchEvent(changed);
        assert.equal(root.lang, 'es', 'A second tab can update the preference');
        stop();
        saved = 'ru';
        events.dispatchEvent(changed);
        assert.equal(root.lang, 'es', 'Teardown removes listeners');
        Object.defineProperty(globalThis, 'localStorage', { configurable: true, get() { throw new Error('storage disabled'); } });
        setLocale('es');
        assert.equal(root.lang, 'es');
        initializeLocale();
        assert.equal(get(locale), 'es', 'Cookie preserves an explicit choice when local storage is disabled');
        setLocale('system');
        assert.equal(get(locale), 'ja');
    } finally {
        stop();
        names.forEach((name, index) => {
            const descriptor = descriptors[index];
            if (descriptor) Object.defineProperty(globalThis, name, descriptor);
            else Reflect.deleteProperty(globalThis, name);
        });
        locale.set('en'); localePreference.set('system');
    }
});

test('every advertised locale contains the complete migrated feature message contracts', () => {
    const placeholders = (value: string) => [...value.matchAll(/\{([A-Za-z][A-Za-z0-9_]*)\}/g)].map(match => match[1]).sort();
    const claimedKeys = new Set<string>();
    for (const catalog of messageCatalogs) {
        const keys = Object.keys(catalog.en).sort();
        for (const key of keys) {
            assert.equal(claimedKeys.has(key), false, `Duplicate catalog key: ${key}`);
            claimedKeys.add(key);
        }
        for (const language of supportedLocales) {
            assert.deepEqual(Object.keys(catalog[language] ?? {}).sort(), keys, language);
            for (const key of keys) {
                assert.ok(catalog[language][key].trim(), `${language}:${key}`);
                assert.deepEqual(placeholders(catalog[language][key]), placeholders(catalog.en[key]), `${language}:${key}`);
            }
        }
    }
});

test('literal translation keys in Svelte pages and controls exist in the registered catalogs', () => {
    const known = new Set(messageCatalogs.flatMap(catalog => Object.keys(catalog.en)));
    const root = new URL('../', import.meta.url);
    const files = ['App.svelte', ...['pages', 'components'].flatMap(directory =>
        readdirSync(new URL(`${directory}/`, root)).filter(file => file.endsWith('.svelte')).map(file => `${directory}/${file}`))];
    let checked = 0;
    for (const file of files) {
        const source = readFileSync(new URL(file, root), 'utf8');
        for (const match of source.matchAll(/\bt\(\$locale,\s*['"]([^'"]+)['"]/g)) {
            assert.ok(known.has(match[1]), `${file}: unknown translation key ${match[1]}`);
            checked++;
        }
    }
    assert.ok(checked > 100, 'Audit must inspect the migrated UI, not an empty file list');
});
