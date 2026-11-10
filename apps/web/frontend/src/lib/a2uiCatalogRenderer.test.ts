import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {iconPath, FALLBACK_ICON} from '../vendor/a2ui-svelte/catalog/basic/icons.ts';
import {renderMarkdown} from '../vendor/a2ui-svelte/catalog/basic/markdown.ts';
import {createServer} from 'vite';

test('A2UI text keeps unsafe URLs inert and escapes model HTML', () => {
    const html = renderMarkdown('[credentials](https://user:password@example.test/path) [script](javascript:alert) <img src=x onerror=evil()>');
    assert.doesNotMatch(html, /<a\b|<img\b|<script\b/);
    assert.match(html, /&lt;img/);
    assert.match(renderMarkdown('[safe](https://example.test/path?q=one&other=two)'), /<a href="https:\/\/example.test\/path\?q=one&amp;other=two"/);
});

test('pending and deleted surfaces have localized status without blank or interactive content', async () => {
    const server = await createServer({server: {middlewareMode: true, ws: false}, appType: 'custom'});
    try {
        const {default: Component} = await server.ssrLoadModule('/src/vendor/a2ui-svelte/render/Surface.svelte');
        const {render} = await server.ssrLoadModule('svelte/server');
        const {locale, supportedLocales, t} = await server.ssrLoadModule('/src/lib/i18n.ts');
        const fixture = JSON.parse(readFileSync(new URL('../../../../../tests/fixtures/a2ui/form_interaction_v0_9_1.json', import.meta.url), 'utf8'));
        for (const language of supportedLocales) {
            locale.set(language);
            for (const phase of ['pending', 'deleted']) {
                const snapshot = {...fixture.initial_snapshot, phase, nodes: {}, root_instance_id: null, summary: ''};
                const html = render(Component, {props: {snapshot, oninteraction: async () => ({status: 'committed'}), onrefresh: async () => {}}}).body;
                assert.ok(html.includes(t(language, `a2ui.${phase}`)));
                assert.match(html, /role="status"/);
                assert.doesNotMatch(html, /<input|<button/);
            }
        }
    } finally { await server.close(); }
});

test('every pinned v0.9.1 catalog icon has a real renderer glyph', () => {
    const catalog = JSON.parse(readFileSync(new URL('../../../../../third_party/a2ui/spec/specification/v0_9_1/catalogs/basic/catalog.json', import.meta.url), 'utf8'));
    const icon = catalog.components.Icon.allOf.find((entry: any) => entry.properties?.name);
    const names: string[] = icon.properties.name.oneOf.find((entry: any) => Array.isArray(entry.enum)).enum;
    assert.equal(names.length, 59, 'review the pinned catalog before changing supported icon names');
    for (const name of names) {
        const path = iconPath(name);
        assert.notEqual(path, FALLBACK_ICON, `${name} must render its catalog glyph, not a placeholder`);
        assert.match(path, /^[Mm]/, `${name} must contain vector path geometry`);
    }
});
