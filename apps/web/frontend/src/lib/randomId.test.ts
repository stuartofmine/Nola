import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readdirSync, readFileSync} from 'node:fs';
import {join} from 'node:path';
import {randomId} from './randomId.ts';

test('resource IDs work on LAN HTTP without secure-context-only randomUUID', () => {
    const random = {getRandomValues: crypto.getRandomValues.bind(crypto)};
    const ids = Array.from({length: 30}, () => randomId(random));
    assert.equal(new Set(ids).size, ids.length);
    for (const id of ids) assert.match(id, /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
});

test('missing CSPRNG is an error rather than a weak identifier fallback', () => {
    assert.throws(() => randomId({getRandomValues() { throw new Error('CSPRNG unavailable'); }}));
});

test('frontend production sources never call secure-context-only crypto.randomUUID directly', () => {
    const srcRoot = new URL('..', import.meta.url);
    const offenders: string[] = [];
    const walk = (dir: string) => {
        for (const entry of readdirSync(dir, {withFileTypes: true})) {
            const full = join(dir, entry.name);
            if (entry.isDirectory()) {
                if (entry.name === 'vendor') continue;
                walk(full);
                continue;
            }
            if (!/\.(ts|svelte|js)$/.test(entry.name) || entry.name.endsWith('.test.ts') || entry.name === 'randomId.ts') continue;
            if (readFileSync(full, 'utf8').includes('crypto.randomUUID')) offenders.push(full);
        }
    };
    walk(srcRoot.pathname);
    assert.deepEqual(offenders, []);
});
