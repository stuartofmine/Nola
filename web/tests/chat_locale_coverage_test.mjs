import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { supportedLocales, messageCatalogs } from '../frontend/src/lib/i18n.ts';
import { createRequire } from 'node:module';

const placeholders = value => [...value.matchAll(/\{([A-Za-z_][\w]*)\}/g)].map(match => match[1]).sort();
const known = new Set(messageCatalogs.flatMap(catalog => Object.keys(catalog.en)));

test('current chat Svelte literal translation calls resolve in the runtime registry', () => {
    const root = new URL('../frontend/src/', import.meta.url);
    const files = ['pages/Chat.svelte', ...readdirSync(new URL('components/', root))
        .filter(file => file.startsWith('Chat') && file.endsWith('.svelte')).map(file => `components/${file}`)];
    let checked = 0;
    for (const file of files) {
        const source = readFileSync(new URL(file, root), 'utf8');
        for (const match of source.matchAll(/\bt\(\$locale,\s*['"]([^'"]+)['"]/g)) {
            assert.ok(known.has(match[1]), `${file}: missing registered key ${match[1]}`);
            checked++;
        }
    }
    assert.ok(checked > 100, 'Inspect the actual Chat page and components, not an empty fixture');
});

for (const locale of supportedLocales) {
    test(`current runtime ${locale} has complete catalog coverage and matching placeholders`, () => {
        for (const catalog of messageCatalogs) {
            assert.deepEqual(Object.keys(catalog[locale] ?? {}).sort(), Object.keys(catalog.en).sort());
            for (const [key, english] of Object.entries(catalog.en)) {
                const value = catalog[locale][key];
                assert.equal(typeof value, 'string', `${locale}:${key}`);
                assert.ok(value.trim(), `${locale}:${key}`);
                assert.deepEqual(placeholders(value), placeholders(english), `${locale}:${key}`);
            }
        }
    });
}

test('current catalog registration cannot silently shadow a translation key', () => {
    const keys = messageCatalogs.flatMap(catalog => Object.keys(catalog.en));
    assert.equal(keys.length, known.size, 'Duplicate registered keys would be overwritten at runtime');
    const ts = createRequire(new URL('../frontend/package.json', import.meta.url))('typescript');
    const root = new URL('../frontend/src/lib/locales/', import.meta.url);
    for (const file of readdirSync(root).filter(file => file.endsWith('.ts'))) {
        const syntax = ts.createSourceFile(file, readFileSync(new URL(file, root), 'utf8'), ts.ScriptTarget.Latest, true);
        function inspect(node) {
            if (ts.isObjectLiteralExpression(node)) {
                const seen = new Set();
                for (const property of node.properties) {
                    const name = property.name;
                    if (!name || !(ts.isIdentifier(name) || ts.isStringLiteral(name) || ts.isNumericLiteral(name))) continue;
                    assert.equal(seen.has(name.text), false, `${file}: duplicate source key ${name.text}`);
                    seen.add(name.text);
                }
            }
            ts.forEachChild(node, inspect);
        }
        inspect(syntax);
    }
});
