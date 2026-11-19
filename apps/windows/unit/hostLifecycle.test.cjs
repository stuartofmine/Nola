// SPDX-License-Identifier: Apache-2.0
const { test } = require('node:test');
const assert = require('node:assert/strict');
const os = require('node:os');
const { HostController } = require('../dist/host.js');

test('missing CLI path transitions HostController to failed state before rejecting', async () => {
    const states = [];
    const controller = new HostController({
        cliPath: '/nonexistent/nola-missing-binary',
        webRoot: os.tmpdir(),
        workspaceRoot: os.tmpdir(),
        log: () => {},
        onState: (state) => states.push(state),
    });

    await assert.rejects(() => controller.start(), /Nola CLI was not found/);
    assert.equal(states.length, 1);
    assert.equal(states[0].kind, 'failed');
    assert.match(states[0].detail, /Nola CLI was not found/);
});

test('missing web root transitions HostController to failed state before rejecting', async () => {
    const states = [];
    const controller = new HostController({
        cliPath: process.execPath,
        webRoot: '/nonexistent/nola-missing-web-root',
        workspaceRoot: os.tmpdir(),
        log: () => {},
        onState: (state) => states.push(state),
    });

    await assert.rejects(() => controller.start(), /Web resources were not found/);
    assert.equal(states.length, 1);
    assert.equal(states[0].kind, 'failed');
    assert.match(states[0].detail, /Web resources were not found/);
});
