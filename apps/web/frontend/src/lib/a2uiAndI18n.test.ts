import test from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { readFileSync, readdirSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { get } from 'svelte/store';
import { applyUiSurfaceUpdate, uiIdentityKey, uiSurfaceReference } from './generativeUi.ts';
import {
    initializeLocale,
    locale,
    localeFromCookie,
    localePreference,
    messageCatalogs,
    normalizeLocale,
    observeLocaleChanges,
    resolveLocale,
    setLocale,
    supportedLocales,
    t,
} from './i18n.ts';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../../../..');
const fixture = JSON.parse(readFileSync(new URL('../../../../../tests/fixtures/a2ui/form_interaction_v0_9_1.json', import.meta.url), 'utf8'));

test('A2UI protocol and renderer references match pinned source digests', () => {
    const reference = path.join(root, 'third_party/a2ui');
    const lock = JSON.parse(readFileSync(path.join(reference, 'sources.lock.json'), 'utf8'));
    assert.equal(lock.wire_version, 'v0.9.1');
    assert.equal(lock.sources.length, 2);
    for (const source of lock.sources) {
        assert.match(source.commit, /^[a-f0-9]{40}$/);
        assert.ok(source.files.some((file: { path: string }) => file.path === 'LICENSE'));
        for (const file of source.files) {
            const location = path.resolve(reference, source.name, file.path);
            assert.ok(location.startsWith(path.resolve(reference, source.name) + path.sep));
            assert.equal(
                createHash('sha256').update(readFileSync(location)).digest('hex'),
                file.sha256,
                `${source.name}/${file.path} changed from its pinned reference`,
            );
        }
    }
});

test('A2UI production renderer excludes upstream runtime/transport and matches adaptation manifest', () => {
    const reference = path.join(root, 'third_party/a2ui');
    const vendor = path.join(root, 'apps/web/frontend/src/vendor/a2ui-svelte');
    const files = readdirSync(vendor, { recursive: true, withFileTypes: true })
        .filter(entry => entry.isFile())
        .map(entry => path.join(entry.parentPath, entry.name));
    assert.ok(files.some(file => file.endsWith('render/Surface.svelte')));
    assert.equal(
        readFileSync(path.join(vendor, 'LICENSE'), 'utf8'),
        readFileSync(path.join(root, 'third_party/a2ui/svelte/LICENSE'), 'utf8'),
    );
    for (const file of files.filter(file => /\.(ts|svelte)$/.test(file))) {
        const text = readFileSync(file, 'utf8');
        assert.doesNotMatch(path.relative(vendor, file), /^(protocol|transport)\//);
        assert.doesNotMatch(
            text,
            /\bnew\s+A2uiClient\b|from\s+['"][^'"]*(?:client\.svelte|protocol\/(?:reducer|resolve|functions)|transport\/)/,
            `${file} imports an independent upstream runtime`,
        );
    }

    const manifest = JSON.parse(readFileSync(path.join(reference, 'production-adaptation.json'), 'utf8'));
    const regenerated = JSON.parse(
        execFileSync(
            'python3',
            ['-c', "import json,runpy; m,p=runpy.run_path('scripts/update_a2ui_adaptation.py')['artifacts'](); print(json.dumps({'manifest':m,'patch':p.decode()}))"],
            { cwd: root, encoding: 'utf8' },
        ),
    );
    assert.deepEqual(regenerated.manifest, manifest);
    assert.equal(regenerated.patch, readFileSync(path.join(reference, manifest.patch), 'utf8'));
    const lock = JSON.parse(readFileSync(path.join(reference, 'sources.lock.json'), 'utf8'));
    assert.equal(manifest.wire_version, 'v0.9.1');
    assert.equal(manifest.upstream_commit, lock.sources.find((source: { name: string }) => source.name === 'svelte').commit);

    const manifestVendor = path.join(root, manifest.production_root);
    const actual = readdirSync(manifestVendor, { recursive: true, withFileTypes: true })
        .filter(entry => entry.isFile())
        .map(entry => path.relative(manifestVendor, path.join(entry.parentPath, entry.name)))
        .sort();
    assert.deepEqual(manifest.files.map((file: { path: string }) => file.path).sort(), actual);
    for (const file of manifest.files) {
        const target = path.resolve(manifestVendor, file.path);
        assert.ok(target.startsWith(manifestVendor + path.sep));
        assert.equal(createHash('sha256').update(readFileSync(target)).digest('hex'), file.sha256, file.path);
        if (file.source) {
            assert.equal(createHash('sha256').update(readFileSync(path.join(root, file.source))).digest('hex'), file.source_sha256);
        }
    }
    assert.equal(createHash('sha256').update(readFileSync(path.join(reference, manifest.patch))).digest('hex'), manifest.patch_sha256);
});

test('A2UI golden projection applies exact delta, fences foreign owners, and preserves read-only history', () => {
    assert.equal(fixture.fixture_version, 1);
    const initial = structuredClone(fixture.initial_snapshot);
    const before = JSON.stringify(initial);
    const updated = applyUiSurfaceUpdate(fixture.identity, initial, { type: 'changes', changes: fixture.changes });
    assert.deepEqual(updated, fixture.edited_snapshot);
    assert.equal(JSON.stringify(initial), before);

    for (const key of Object.keys(fixture.identity.owner)) {
        const foreign = structuredClone(fixture.identity);
        foreign.owner[key] += '-foreign';
        assert.throws(() => applyUiSurfaceUpdate(foreign, fixture.initial_snapshot, { type: 'changes', changes: fixture.changes }));
    }
    assert.throws(() => applyUiSurfaceUpdate(fixture.identity, fixture.edited_snapshot, { type: 'changes', changes: fixture.changes }));

    const historical = fixture.historical_snapshot;
    assert.equal(historical.phase, 'read_only');
    assert.equal(uiIdentityKey(historical.identity), uiIdentityKey(fixture.identity));
    for (const node of Object.values(historical.nodes) as Array<{ actions: object; bindings: object }>) {
        assert.deepEqual(node.actions, {});
        assert.deepEqual(node.bindings, {});
    }
    const surface = { identity: fixture.identity, revision: fixture.initial_snapshot.revision };
    assert.deepEqual(
        uiSurfaceReference({ kind: 'ui_surface', surface }, fixture.identity.owner.conversation_id, fixture.identity.owner.message_id),
        surface,
    );
});

test('i18n resolves 7 locales, validates cookies, persists preference, and covers all catalog keys', () => {
    assert.equal(supportedLocales.length, 7);
    assert.equal(resolveLocale('ja', ['zh-CN', 'en-US']), 'ja');
    assert.equal(resolveLocale('system', ['xx', 'fr-CA', 'en']), 'fr');
    assert.equal(resolveLocale('invalid', ['de-DE']), 'de');
    assert.equal(resolveLocale(null, ['unknown']), 'en');
    assert.equal(normalizeLocale('ZH_cn'), 'zh-Hans');
    assert.equal(normalizeLocale('pt-BR'), undefined);
    assert.equal(t('en', 'models.configureTitle', { name: '<script>{name}</script>' }), '<script>{name}</script> local model configuration');
    assert.equal(t('en', 'missing.key'), 'missing.key');

    assert.equal(localeFromCookie('nola_web_session=secret; nola_ui_locale=ja'), 'ja');
    for (const cookie of ['other_nola_ui_locale=ja', 'nola_ui_locale=zh-CN', 'nola_ui_locale=system', 'nola_ui_locale=ja; nola_ui_locale=de']) {
        assert.equal(localeFromCookie(cookie), undefined);
    }

    const names = ['localStorage', 'document', 'navigator', 'location'] as const;
    const descriptors = names.map(name => Object.getOwnPropertyDescriptor(globalThis, name));
    let saved = 'ja';
    const docRoot = { lang: '' };
    const browser = { languages: ['fr-CA'] };
    const events = new EventTarget();
    const stop = observeLocaleChanges(events as Window);
    try {
        Object.defineProperty(globalThis, 'localStorage', {
            configurable: true,
            value: { getItem: () => saved, setItem: (_key: string, value: string) => { saved = value; } },
        });
        Object.defineProperty(globalThis, 'document', { configurable: true, value: { documentElement: docRoot } });
        Object.defineProperty(globalThis, 'navigator', { configurable: true, value: browser });
        Object.defineProperty(globalThis, 'location', { configurable: true, value: { protocol: 'https:' } });
        initializeLocale();
        assert.equal(get(locale), 'ja');
        setLocale('de');
        assert.equal(saved, 'de');
        assert.equal(docRoot.lang, 'de');
        setLocale('system');
        assert.equal(get(localePreference), 'system');
        assert.equal(docRoot.lang, 'fr');
    } finally {
        stop();
        names.forEach((name, index) => {
            const descriptor = descriptors[index];
            if (descriptor) Object.defineProperty(globalThis, name, descriptor);
            else Reflect.deleteProperty(globalThis, name);
        });
        locale.set('en');
        localePreference.set('system');
    }

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

    const srcRoot = new URL('../', import.meta.url);
    const svelteFiles = [
        'App.svelte',
        ...['pages', 'components'].flatMap(directory =>
            readdirSync(new URL(`${directory}/`, srcRoot))
                .filter(file => file.endsWith('.svelte'))
                .map(file => `${directory}/${file}`)),
    ];
    let checked = 0;
    for (const file of svelteFiles) {
        const source = readFileSync(new URL(file, srcRoot), 'utf8');
        for (const match of source.matchAll(/\bt\(\$locale,\s*['"]([^'"]+)['"]/g)) {
            assert.ok(claimedKeys.has(match[1]), `${file}: unknown translation key ${match[1]}`);
            checked++;
        }
    }
    assert.ok(checked > 100);
});
