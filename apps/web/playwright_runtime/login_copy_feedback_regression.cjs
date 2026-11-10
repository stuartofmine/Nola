// SPDX-License-Identifier: Apache-2.0
// Mount production components in Chromium; no provider or local account access.
const {chromium} = require('playwright-core');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const {pathToFileURL} = require('node:url');

(async () => {
    const root = path.resolve(__dirname, '../frontend');
    const fixture = fs.mkdtempSync(path.join(root, '.interaction-fixture-'));
    let server, browser;
    try {
        fs.writeFileSync(path.join(fixture, 'index.html'), '<div id="app"></div><script type="module" src="./main.js"></script>');
        fs.writeFileSync(path.join(fixture, 'main.js'), "import {mount} from 'svelte'; import App from './App.svelte'; import {initializeLocale} from '../src/lib/i18n'; initializeLocale(); mount(App,{target:document.querySelector('#app')});");
        fs.writeFileSync(path.join(fixture, 'App.svelte'), `<script>
            import CodexSourceLogin from '../src/components/CodexSourceLogin.svelte';
            import Markdown from '../src/components/Markdown.svelte';
            import '../src/theme.css';
            let source = $state('initial');
            let showLogin = $state(true);
            window.updateMarkdown = value => source = value;
            window.unmountLogin = () => showLogin = false;
        </script>{#if showLogin}<CodexSourceLogin />{/if}<Markdown {source} streaming={true} />`);
        const {createServer} = await import(pathToFileURL(path.join(root, 'node_modules/vite/dist/node/index.js')).href);
        server = await createServer({root, server: {host: '127.0.0.1', port: 0}, logLevel: 'error'});
        await server.listen();
        const base = `http://127.0.0.1:${server.httpServer.address().port}`;
        browser = await chromium.launch({...require('./test_environment.cjs').browserOptions(), headless: true});
        for (const scheme of ['light', 'dark']) for (const width of [320, 1440]) {
            const context = await browser.newContext({viewport: {width, height: 900}, colorScheme: scheme, locale: 'en-US', reducedMotion: 'reduce'});
            const page = await context.newPage();
            page.setDefaultTimeout(5000);
            const errors = []; page.on('pageerror', error => errors.push(error.message));
            await page.addInitScript(() => {
                // Accelerate only the production request deadline, not polling or feedback.
                const timeout = AbortSignal.timeout.bind(AbortSignal);
                AbortSignal.timeout = ms => timeout(ms === 15000 ? 600 : ms);
                window.copies = [];
                Object.defineProperty(navigator, 'clipboard', {configurable: true, value: {writeText: text => new Promise((resolve, reject) => window.copies.push({text, resolve, reject}))}});
            });
            let starts = 0, statuses = 0, cancels = 0, captured, held, heldStart;
            let mode = 'lost-start';
            const snapshot = state => ({request_id: captured.request_id, state, authorization_url: state === 'pending' ? 'https://auth.openai.com/authorize' : null, inference_supported: false, account: null});
            await page.route('**/*', async route => {
                const url = new URL(route.request().url());
                if (url.origin !== base) return route.abort();
                if (!url.pathname.startsWith('/api/')) return route.continue();
                if (url.pathname === '/api/model-connections') return route.fulfill({json: {accounts: []}});
                if (url.pathname.endsWith('/start')) {
                    starts++; captured = route.request().postDataJSON();
                    assert.equal(captured.display_name, 'Fixture'); assert.equal(captured.executable, '/fixtures/codex');
                    assert.equal(captured.codex_home, '/fixtures/home');
                    if (mode === 'start-hold') { heldStart = route; return; }
                    return route.fulfill({status: 503, json: {error: 'Lost fixture start receipt'}});
                }
                if (url.pathname.endsWith('/status')) {
                    statuses++;
                    if (mode === 'hold') { held = route; return; }
                    if (mode === 'timeout') return;
                    return route.fulfill({json: snapshot('pending')});
                }
                if (url.pathname.endsWith('/cancel')) { cancels++; return route.fulfill({json: snapshot('cancelled')}); }
                return route.fulfill({status: 503, json: {error: 'No fixture'}});
            });
            await page.goto(`${base}/ui/${path.basename(fixture)}/index.html`);
            await page.locator('summary').click();
            const inputs = page.locator('details input:not([type=checkbox])');
            await inputs.nth(0).fill('   '); await inputs.nth(1).fill('/fixtures/codex'); await inputs.nth(2).fill('/fixtures/home');
            await page.locator('input[type=checkbox]').check();
            const start = page.getByRole('button', {name: 'Start Codex sign-in', exact: true});
            assert.equal(await start.isDisabled(), true, 'blank labels cannot start');
            await inputs.nth(0).fill(' Fixture '); await inputs.nth(1).fill(' /fixtures/codex '); await inputs.nth(2).fill(' /fixtures/home ');
            await start.click();
            const retry = page.getByRole('button', {name: 'Check this sign-in', exact: true});
            await retry.waitFor(); assert.equal(starts, 1); assert.equal(await start.isDisabled(), true, 'lost receipt must not enable another mutation');
            mode = 'timeout'; await retry.click();
            await page.waitForFunction(() => ![...document.querySelectorAll('button')].find(e => e.textContent === 'Check this sign-in')?.disabled);
            assert.equal(statuses, 1, 'timed-out observation settles for manual recovery');
            mode = 'hold';
            await retry.evaluate(button => { button.click(); button.click(); });
            await page.waitForFunction(() => [...document.querySelectorAll('button')].find(e => e.textContent === 'Check this sign-in')?.disabled);
            assert.equal(statuses, 2, 'only one status request can be in flight');
            await page.getByRole('button', {name: 'Cancel local sign-in', exact: true}).click();
            await page.waitForFunction(() => ![...document.querySelectorAll('button')].find(e => e.textContent === 'Start Codex sign-in')?.disabled);
            await held.fulfill({json: snapshot('pending')});
            await page.waitForTimeout(1100);
            assert.equal(cancels, 1); assert.equal(statuses, 2); assert.equal(starts, 1);
            assert.equal(await page.getByRole('link', {name: 'Continue in OpenAI sign-in'}).count(), 0, 'late pending status cannot revive cancelled login');
            mode = 'start-hold'; await start.click();
            await page.waitForTimeout(50); assert.ok(heldStart);
            await page.evaluate(() => window.unmountLogin());
            await page.waitForTimeout(50); assert.equal(cancels, 2, 'unmount cancels even while start response is outstanding');
            await heldStart.fulfill({json: snapshot('pending')});
            await page.waitForTimeout(50); assert.equal(cancels, 3, 'late creation settles the same cancelled operation');

            const code = text => '```text\n' + text + '\n```';
            await page.evaluate(value => window.updateMarkdown(value), code('first'));
            const copy = page.locator('.code-header button');
            await copy.click();
            await page.evaluate(value => window.updateMarkdown(value), code('second'));
            await page.evaluate(() => window.copies[0].resolve());
            assert.equal(await page.locator('.copy-notice').count(), 0, 'old content cannot acquire a copied notice');
            await copy.click(); await copy.click();
            await page.evaluate(() => window.copies[2].resolve());
            await page.locator('.copy-notice').waitFor();
            const success = await page.locator('.copy-notice').textContent();
            await page.evaluate(() => window.copies[1].reject(new Error('Delayed clipboard rejection')));
            assert.equal(await page.locator('.copy-notice').textContent(), success, 'old failure cannot overwrite newer success');
            await page.locator('.copy-notice').waitFor({state: 'hidden'});
            await copy.click(); await page.evaluate(() => window.copies[3].reject(new Error('Clipboard unavailable')));
            await page.locator('.copy-notice').waitFor(); assert.notEqual(await page.locator('.copy-notice').textContent(), success);
            assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true, 'no narrow overflow');
            assert.deepEqual(errors, []);
            if (process.env.UI_EVIDENCE_DIR) { fs.mkdirSync(process.env.UI_EVIDENCE_DIR, {recursive: true}); await page.screenshot({path: path.join(process.env.UI_EVIDENCE_DIR, `login-copy-${width}-${scheme}.png`)}); }
            await context.close();
        }
        console.log('PASS: four viewport/theme combinations; login timeout, deduplicated reads, cancellation ownership, whitespace validation, stale clipboard completion, feedback expiry and failure');
    } finally {
        await browser?.close(); await server?.close(); fs.rmSync(fixture, {recursive: true, force: true});
    }
})().catch(error => { console.error(error); process.exitCode = 1; });
