// SPDX-License-Identifier: Apache-2.0
const { test } = require('node:test');
const assert = require('node:assert/strict');
const { mkdir, mkdtemp, rm, symlink, writeFile } = require('node:fs/promises');
const os = require('node:os');
const path = require('node:path');
const { previousHostPort } = require('../dist/previousHostPort.js');

const timestamp = '2026-03-14T01:02:03.004Z';
const state = origin => timestamp + ' [state] ' + JSON.stringify({ kind: 'ready', origin });
async function profile(t) {
    const root = await mkdtemp(path.join(os.tmpdir(), 'nola-desktop-upgrade-'));
    t.after(() => rm(root, { recursive: true, force: true }));
    const directory = path.join(root, 'logs');
    await mkdir(directory);
    return { root, directory, file: path.join(directory, 'nola-windows.log'), port: path.join(root, 'host-port') };
}

test('recovers only the last ready state, across a read boundary and long diagnostic lines', async t => {
    const { file, port } = await profile(t);
    for (const size of [65502, 65536 * 32 - 40]) {
        const lines = [
            state('http://127.0.0.1:15210'),
            'x'.repeat(210000),
            state('http://127.0.0.1:16211'),
            timestamp + ' [stdout] ' + state('http://127.0.0.1:54321'),
            timestamp + ' [state] {"kind":"failed","detail":"http://127.0.0.1:59876"}',
            'x'.repeat(size),
        ];
        await writeFile(file, lines.join('\r\n') + '\r\n');
        assert.equal(await previousHostPort(port), 16211);
    }
    // Split the actual state record across two reads.
    const ready = state('http://127.0.0.1:18441');
    await writeFile(file, 'old\n' + ready + '\n' + 'x'.repeat(65536 - 26) + '\n');
    assert.equal(await previousHostPort(port), 18441);
});

test('unattributable latest ready records do not select an older origin', async t => {
    const { file, port } = await profile(t);
    for (const next of ['https://127.0.0.1:3456', 'http://localhost:3456', 'http://127.0.0.2:3456',
        'http://127.0.0.1:65536', 'http://127.0.0.1:01234', 'http://127.0.0.1:0',
        'http://user@127.0.0.1:3456', 'http://127.0.0.1:3456/path', 'http://127.0.0.1:3456?debug=1']) {
        await writeFile(file, state('http://127.0.0.1:12345') + '\n' + state(next) + '\n');
        assert.equal(await previousHostPort(port), 0);
    }
    await writeFile(file, state('http://127.0.0.1:12345') + '\n' + state('http://127.0.0.1:3456'));
    assert.equal(await previousHostPort(port), 0, 'an interrupted ready record cannot establish a new origin');
    await writeFile(file, timestamp + ' [stdout] ' + state('http://127.0.0.1:54321') + '\n');
    assert.equal(await previousHostPort(port), 0);
});

test('an empty profile never inherits an adjacent profile or its linked log directory', async t => {
    const { directory, file, port } = await profile(t);
    const other = await profile(t);
    await writeFile(other.file, state('http://127.0.0.1:49100') + '\n');
    assert.equal(await previousHostPort(port), 0);
    await rm(directory, { recursive: true });
    await symlink(other.directory, directory, 'junction');
    assert.equal(await previousHostPort(port), 0);
    assert.equal(require('node:fs').readFileSync(file, 'utf8'), state('http://127.0.0.1:49100') + '\n');
});
