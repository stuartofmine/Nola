import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { readFileSync, readdirSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../../..');

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
            assert.equal(createHash('sha256').update(readFileSync(location)).digest('hex'), file.sha256,
                `${source.name}/${file.path} changed from its pinned reference`);
        }
    }
});

test('production renderer does not retain upstream runtime or transport modules', () => {
    const vendor = path.join(root, 'web/frontend/src/vendor/a2ui-svelte');
    const files = readdirSync(vendor, { recursive: true, withFileTypes: true })
        .filter(entry => entry.isFile()).map(entry => path.join(entry.parentPath, entry.name));
    assert.ok(files.some(file => file.endsWith('render/Surface.svelte')));
    assert.equal(readFileSync(path.join(vendor, 'LICENSE'), 'utf8'),
        readFileSync(path.join(root, 'third_party/a2ui/svelte/LICENSE'), 'utf8'));
    for (const file of files.filter(file => /\.(ts|svelte)$/.test(file))) {
        const text = readFileSync(file, 'utf8');
        assert.doesNotMatch(path.relative(vendor, file), /^(protocol|transport)\//);
        assert.doesNotMatch(text, /\bnew\s+A2uiClient\b|from\s+['"][^'"]*(?:client\.svelte|protocol\/(?:reducer|resolve|functions)|transport\/)/,
            `${file} imports an independent upstream runtime`);
    }
});

test('production adaptation inventory and patch reproduce the pinned renderer', () => {
    const reference = path.join(root, 'third_party/a2ui');
    const manifest = JSON.parse(readFileSync(path.join(reference, 'production-adaptation.json'), 'utf8'));
    const regenerated = JSON.parse(execFileSync('python3', ['-c', "import json,runpy; m,p=runpy.run_path('scripts/update_a2ui_adaptation.py')['artifacts'](); print(json.dumps({'manifest':m,'patch':p.decode()}))"], {cwd:root, encoding:'utf8'}));
    assert.deepEqual(regenerated.manifest, manifest);
    assert.equal(regenerated.patch, readFileSync(path.join(reference, manifest.patch), 'utf8'));
    const lock = JSON.parse(readFileSync(path.join(reference, 'sources.lock.json'), 'utf8'));
    assert.equal(manifest.wire_version, 'v0.9.1');
    assert.equal(manifest.upstream_commit, lock.sources.find((source: {name: string}) => source.name === 'svelte').commit);
    const vendor = path.join(root, manifest.production_root);
    const actual = readdirSync(vendor, {recursive:true, withFileTypes:true}).filter(entry => entry.isFile())
        .map(entry => path.relative(vendor, path.join(entry.parentPath, entry.name))).sort();
    assert.deepEqual(manifest.files.map((file: {path: string}) => file.path).sort(), actual);
    for (const file of manifest.files) {
        const target = path.resolve(vendor, file.path);
        assert.ok(target.startsWith(vendor + path.sep));
        assert.equal(createHash('sha256').update(readFileSync(target)).digest('hex'), file.sha256, file.path);
        if (file.source) assert.equal(createHash('sha256').update(readFileSync(path.join(root,file.source))).digest('hex'), file.source_sha256);
    }
    assert.equal(createHash('sha256').update(readFileSync(path.join(reference,manifest.patch))).digest('hex'), manifest.patch_sha256);
});
