// SPDX-License-Identifier: Apache-2.0
// Import the current runtime registry; no second TypeScript/catalog evaluator.
import { readdirSync, readFileSync } from 'node:fs';
import { join, relative, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { createRequire } from 'node:module';

const sourceRoot = join(resolve(process.argv[2]), 'web/frontend/src');
const require = createRequire(pathToFileURL(join(sourceRoot, '../package.json')));
const ts = require('typescript');
const { supportedLocales, messageCatalogs } = await import(pathToFileURL(join(sourceRoot, 'lib/i18n.ts')).href);
const sourceKeys = [];
const duplicateKeys = [];
let sourceFiles = 0;
function visit(directory) {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
        const path = join(directory, entry.name);
        if (entry.isDirectory()) { visit(path); continue; }
        if (!/\.(?:svelte|ts)$/.test(entry.name) || entry.name.endsWith('.test.ts')) continue;
        sourceFiles++;
        const source = readFileSync(path, 'utf8');
        if (path.startsWith(join(sourceRoot, 'lib/locales/'))) {
            // Use the installed compiler's grammar, not regex/string matching
            // that would hide duplicate keys before runtime Object.fromEntries.
            const syntax = ts.createSourceFile(path, source, ts.ScriptTarget.Latest, true);
            function inspect(node) {
                if (ts.isObjectLiteralExpression(node)) {
                    const keys = new Set();
                    for (const property of node.properties) {
                        const name = property.name;
                        if (!name || !(ts.isIdentifier(name) || ts.isStringLiteral(name) || ts.isNumericLiteral(name))) continue;
                        if (keys.has(name.text)) duplicateKeys.push({ key: name.text, file: relative(sourceRoot, path) });
                        keys.add(name.text);
                    }
                }
                ts.forEachChild(node, inspect);
            }
            inspect(syntax);
        }
        for (const match of source.matchAll(/\bt\(\s*(?:\$?locale|language|get\(locale\))\s*,\s*(['"])([^'"\\]+)\1/g)) {
            sourceKeys.push({ key: match[2], file: relative(sourceRoot, path) });
        }
    }
}
visit(sourceRoot);
process.stdout.write(JSON.stringify({ locales: supportedLocales, catalogs: messageCatalogs, sourceKeys, sourceFiles, duplicateKeys }));
