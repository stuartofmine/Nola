// SPDX-License-Identifier: Apache-2.0
const { test } = require('node:test');
const assert = require('node:assert/strict');
const { chmodSync, existsSync, mkdtempSync, rmSync, writeFileSync } = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { HostController, resolveExecutable } = require('../dist/host.js');

const repoRoot = path.resolve(__dirname, '../../..');

function createFakeHostBinary(dir, scriptBody) {
    const jsPath = path.join(dir, 'fake-host.cjs');
    const shPath = path.join(dir, 'fake-nola');
    writeFileSync(jsPath, scriptBody, 'utf8');
    writeFileSync(shPath, `#!/bin/sh\nexec ${JSON.stringify(process.execPath)} ${JSON.stringify(jsPath)} "$@"\n`, 'utf8');
    chmodSync(shPath, 0o755);
    return shPath;
}

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

test('HostController transitions starting -> ready, verifies args/env, and stops cleanly via SIGTERM', async () => {
    const tmpDir = mkdtempSync(path.join(os.tmpdir(), 'nola-win-host-ready-'));
    try {
        const recordFile = path.join(tmpDir, 'invocation.json');
        const cliPath = createFakeHostBinary(tmpDir, `
            const http = require('node:http');
            const fs = require('node:fs');
            const args = process.argv.slice(2);
            const port = Number(args[args.indexOf('--port') + 1]);
            fs.writeFileSync(${JSON.stringify(recordFile)}, JSON.stringify({
                args,
                platform: process.env.NOLA_DESKTOP_PLATFORM,
                pid: process.pid,
            }));
            const server = http.createServer((req, res) => {
                if (req.url === '/api/health') { res.writeHead(200); res.end('ok'); return; }
                res.writeHead(404); res.end();
            });
            server.listen(port, '127.0.0.1');
            process.on('SIGTERM', () => { server.close(() => process.exit(0)); });
        `);

        const states = [];
        const controller = new HostController({
            cliPath,
            webRoot: tmpDir,
            workspaceRoot: tmpDir,
            maxAttempts: 1,
            log: () => {},
            onState: (state) => states.push(state),
        });

        const origin = await controller.start();
        try {
            assert.match(origin, /^http:\/\/127\.0\.0\.1:\d+$/);
            assert.deepEqual(states, [{ kind: 'starting', attempt: 1 }, { kind: 'ready', origin }]);
            const health = await fetch(`${origin}/api/health`);
            assert.equal(health.status, 200);
            const info = JSON.parse(require('node:fs').readFileSync(recordFile, 'utf8'));
            assert.equal(info.platform, 'windows');
            assert.deepEqual(info.args.slice(0, 5), ['web', '--host', '127.0.0.1', '--port', String(new URL(origin).port)]);
        } finally {
            await controller.stop();
        }
        assert.equal(states.at(-1).kind, 'stopped');
        await assert.rejects(() => fetch(`${origin}/api/health`, { signal: AbortSignal.timeout(500) }));
    } finally {
        rmSync(tmpDir, { recursive: true, force: true });
    }
});

test('HostController retries with backoff across maxAttempts and fails when all attempts fail', async () => {
    const tmpDir = mkdtempSync(path.join(os.tmpdir(), 'nola-win-host-retry-'));
    try {
        const counterFile = path.join(tmpDir, 'count.txt');
        const cliPath = createFakeHostBinary(tmpDir, `
            const fs = require('node:fs');
            const http = require('node:http');
            const counterFile = ${JSON.stringify(counterFile)};
            const count = (fs.existsSync(counterFile) ? Number(fs.readFileSync(counterFile, 'utf8')) : 0) + 1;
            fs.writeFileSync(counterFile, String(count));
            if (count === 1) process.exit(17);
            const args = process.argv.slice(2);
            const port = Number(args[args.indexOf('--port') + 1]);
            const server = http.createServer((req, res) => { res.writeHead(200); res.end('ok'); });
            server.listen(port, '127.0.0.1');
            process.on('SIGTERM', () => server.close(() => process.exit(0)));
        `);

        const states = [];
        const startedAt = Date.now();
        const controller = new HostController({
            cliPath,
            webRoot: tmpDir,
            workspaceRoot: tmpDir,
            maxAttempts: 2,
            log: () => {},
            onState: (state) => states.push(state),
        });

        const origin = await controller.start();
        try {
            const elapsedMs = Date.now() - startedAt;
            assert.ok(elapsedMs >= 450, `Expected attempt 1 backoff >= 450ms, got ${elapsedMs}ms`);
            assert.deepEqual(states, [
                { kind: 'starting', attempt: 1 },
                { kind: 'starting', attempt: 2 },
                { kind: 'ready', origin },
            ]);
        } finally {
            await controller.stop();
        }

        // Now verify total failure across maxAttempts=2
        const failingCli = createFakeHostBinary(tmpDir, 'process.exit(23);');
        const failStates = [];
        const failingController = new HostController({
            cliPath: failingCli,
            webRoot: tmpDir,
            workspaceRoot: tmpDir,
            maxAttempts: 2,
            log: () => {},
            onState: (state) => failStates.push(state),
        });
        await assert.rejects(() => failingController.start(), /exited with code 23/);
        assert.equal(failStates[0].kind, 'starting');
        assert.equal(failStates[1].kind, 'starting');
        assert.equal(failStates[2].kind, 'failed');
    } finally {
        rmSync(tmpDir, { recursive: true, force: true });
    }
});

test('HostController transitions to failed on post-ready crash and reallocates port on restart', async () => {
    const tmpDir = mkdtempSync(path.join(os.tmpdir(), 'nola-win-host-crash-'));
    try {
        const cliPath = createFakeHostBinary(tmpDir, `
            const http = require('node:http');
            const args = process.argv.slice(2);
            const port = Number(args[args.indexOf('--port') + 1]);
            const server = http.createServer((req, res) => { res.writeHead(200); res.end('ok'); });
            server.listen(port, '127.0.0.1');
            setTimeout(() => { server.close(() => process.exit(42)); }, 250);
            process.on('SIGTERM', () => server.close(() => process.exit(0)));
        `);

        const states = [];
        const controller = new HostController({
            cliPath,
            webRoot: tmpDir,
            workspaceRoot: tmpDir,
            maxAttempts: 1,
            log: () => {},
            onState: (state) => states.push(state),
        });

        const origin1 = await controller.start();
        await new Promise((resolve) => setTimeout(resolve, 600));
        const crashed = states.find((s) => s.kind === 'failed');
        assert.ok(crashed, `Expected post-ready crash state, got: ${JSON.stringify(states)}`);
        assert.match(crashed.detail, /code=42/);

        const origin2 = await controller.restart();
        try {
            assert.notEqual(origin2, origin1, 'restart() should allocate a fresh loopback port');
            assert.equal((await fetch(`${origin2}/api/health`)).status, 200);
        } finally {
            await controller.stop();
        }
    } finally {
        rmSync(tmpDir, { recursive: true, force: true });
    }
});

test('resolveExecutable prioritizes override, then packaged, then development candidates', () => {
    const tmpDir = mkdtempSync(path.join(os.tmpdir(), 'nola-win-resolve-'));
    try {
        const override = path.join(tmpDir, 'override');
        const packaged = path.join(tmpDir, 'packaged');
        const devA = path.join(tmpDir, 'dev-a');
        const devB = path.join(tmpDir, 'dev-b');
        writeFileSync(devB, '');
        assert.equal(resolveExecutable({ packagedCandidate: packaged, developmentCandidates: [devA, devB] }), path.resolve(devB));
        writeFileSync(packaged, '');
        assert.equal(resolveExecutable({ override: path.join(tmpDir, 'missing'), packagedCandidate: packaged, developmentCandidates: [devA, devB] }), path.resolve(packaged));
        writeFileSync(override, '');
        assert.equal(resolveExecutable({ override, packagedCandidate: packaged, developmentCandidates: [devA, devB] }), path.resolve(override));
        assert.equal(resolveExecutable({ packagedCandidate: path.join(tmpDir, 'none1'), developmentCandidates: [path.join(tmpDir, 'none2')] }), path.resolve(path.join(tmpDir, 'none1')));
    } finally {
        rmSync(tmpDir, { recursive: true, force: true });
    }
});

const realNola = path.join(repoRoot, 'target/debug/nola');
test('HostController launches and stops real target/debug/nola binary in an isolated temp home', { skip: !existsSync(realNola) }, async () => {
    const tmpDir = mkdtempSync(path.join(os.tmpdir(), 'nola-win-real-host-'));
    const prevEnv = {
        HOME: process.env.HOME,
        NOLA_HOME: process.env.NOLA_HOME,
        NOLA_DB_PATH: process.env.NOLA_DB_PATH,
        NOLA_MAC_APP_SUPPORT: process.env.NOLA_MAC_APP_SUPPORT,
    };
    const homeDir = path.join(tmpDir, 'home');
    require('node:fs').mkdirSync(homeDir, { recursive: true });
    process.env.HOME = homeDir;
    process.env.NOLA_HOME = homeDir;
    process.env.NOLA_DB_PATH = path.join(tmpDir, 'nola.db');
    delete process.env.NOLA_MAC_APP_SUPPORT;

    const states = [];
    const controller = new HostController({
        cliPath: realNola,
        webRoot: path.join(repoRoot, 'apps/web'),
        workspaceRoot: tmpDir,
        maxAttempts: 1,
        log: () => {},
        onState: (state) => states.push(state),
    });

    try {
        const origin = await controller.start();
        const res = await fetch(`${origin}/api/health`);
        assert.equal(res.status, 200);
        const session = await (await fetch(`${origin}/api/web/session`)).json();
        assert.equal(session.localTrust, true);
        assert.equal(states.at(-1).kind, 'ready');
    } finally {
        await controller.stop();
        for (const [k, v] of Object.entries(prevEnv)) {
            if (v === undefined) delete process.env[k];
            else process.env[k] = v;
        }
        rmSync(tmpDir, { recursive: true, force: true });
    }
});
