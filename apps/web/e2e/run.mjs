import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(__dirname, '../../..');
const WEB_ROOT = path.join(REPO_ROOT, 'apps/web');
const FRONTEND_DIR = path.join(WEB_ROOT, 'frontend');
const STATIC_INDEX = path.join(WEB_ROOT, 'app/static/ui/index.html');
const NOLA_BIN = process.env.NOLA_BIN || path.join(REPO_ROOT, 'target/debug/nola');
const PLAYWRIGHT_CLI = path.join(REPO_ROOT, 'apps/windows/node_modules/playwright/cli.js');
const CONFIG_PATH = path.join(__dirname, 'playwright.config.mjs');

function runOrFail(cmd, args, opts = {}) {
    const result = spawnSync(cmd, args, {
        stdio: 'inherit',
        cwd: REPO_ROOT,
        ...opts,
    });
    if (result.status !== 0) {
        process.exit(result.status ?? 1);
    }
}

if (!fs.existsSync(PLAYWRIGHT_CLI)) {
    console.error(`Playwright CLI not found at ${PLAYWRIGHT_CLI}`);
    process.exit(1);
}

if (!fs.existsSync(STATIC_INDEX) || process.env.NOLA_E2E_REBUILD_FRONTEND === '1') {
    console.log('[web-e2e] Building frontend assets...');
    runOrFail('npm', ['--prefix', FRONTEND_DIR, 'run', 'build']);
}

if (!fs.existsSync(NOLA_BIN)) {
    console.log('[web-e2e] Building nola debug binary...');
    runOrFail('cargo', ['build', '--locked', '-p', 'nola-app', '--bin', 'nola']);
}

const extraArgs = process.argv.slice(2);
console.log('[web-e2e] Running Playwright real-browser E2E suite...');
runOrFail(process.execPath, [PLAYWRIGHT_CLI, 'test', '-c', CONFIG_PATH, ...extraArgs], {
    env: {
        ...process.env,
        NOLA_BIN,
    },
});
