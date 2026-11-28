import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const CHROME_MAC = '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const CHROME_EXEC = process.env.NOLA_E2E_CHROME_PATH
    || (fs.existsSync(CHROME_MAC) ? CHROME_MAC : undefined);

export default {
    testDir: __dirname,
    testMatch: '**/*.spec.mjs',
    outputDir: path.join(os.tmpdir(), 'nola-web-e2e-results'),
    fullyParallel: false,
    workers: 1,
    retries: 0,
    timeout: 90_000,
    expect: {
        timeout: 20_000,
    },
    reporter: [['list']],
    use: {
        browserName: 'chromium',
        headless: process.env.NOLA_E2E_HEADED !== '1',
        viewport: { width: 1440, height: 900 },
        locale: 'en-US',
        launchOptions: {
            ...(CHROME_EXEC ? { executablePath: CHROME_EXEC } : {}),
            args: ['--no-first-run', '--no-default-browser-check'],
        },
    },
};
