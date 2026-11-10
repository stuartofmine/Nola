// SPDX-License-Identifier: Apache-2.0
const assert = require('node:assert/strict');
const {spawnSync} = require('node:child_process');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');
const environment = require('./test_environment.cjs');

const repositoryRoot = path.resolve(__dirname, '../../..');

test('artifact and configured browser paths resolve independently of the caller directory', () => {
    const modulePath = require.resolve('./test_environment.cjs');
    const source = `const e=require(${JSON.stringify(modulePath)}); process.stdout.write(JSON.stringify({artifact:e.artifactPath('portable/report.json'),browser:e.browserOptions({CHROME_PATH:'tools/browser'})}));`;
    const result = spawnSync(process.execPath, ['-e', source], {cwd: os.tmpdir(), encoding: 'utf8'});
    assert.equal(result.status, 0, result.stderr);
    assert.deepEqual(JSON.parse(result.stdout), {
        artifact: path.join(repositoryRoot, '.artifacts/portable/report.json'),
        browser: {executablePath: path.join(repositoryRoot, 'tools/browser')},
    });
});

test('artifact paths cannot escape the ignored evidence directory', () => {
    for (const invalid of ['', '..', '../outside', '/outside', 'C:\\outside']) {
        assert.throws(() => environment.artifactPath(invalid));
    }
});

test('temporary scratch paths follow the platform temporary directory', () => {
    assert.equal(environment.temporaryPrefix('nola-browser-'), path.join(os.tmpdir(), 'nola-browser-'));
    for (const invalid of ['', '.', '..', '../outside', '/outside', 'nested/file']) {
        assert.throws(() => environment.temporaryPrefix(invalid));
    }
});

test('browser discovery uses a channel unless an executable was explicitly configured', () => {
    assert.deepEqual(environment.browserOptions({}), {channel: 'chrome'});
    assert.deepEqual(environment.browserOptions({CHROME_PATH: '  '}), {channel: 'chrome'});
});
