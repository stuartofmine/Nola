// Real isolated Chrome; every mutation is fulfilled locally, never sent to Host.
const { chromium } = require('playwright-core');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { verifyPreviewOwner } = require('./ui_preview_owner.cjs');

async function main() {
    verifyPreviewOwner(process.env.NOLA_UI_PREVIEW_DB);
    const origin = 'http://127.0.0.1:5001';
    const output = fs.mkdtempSync(path.join(os.tmpdir(), 'nola-probe-ui.'));
    const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'nola-probe-chrome.'));
    const report = { checks: [], blocked: [], errors: [] };
    const inventory = { accounts: [{ revision: 7, account: { id: 'fixture-account', display_name: 'Isolated fixture account', source: 'manual', state: 'ready' } }], selections: [], connections: [{ revision: 11, probe_credential_bindings: { openai_responses: 'fixture-binding' }, document: { format_version: 1, connection: { id: 'fixture-connection', account_id: 'fixture-account', display_name: 'Synthetic — no provider access', base_url: 'https://invalid.example/v1', protocols: ['openai_responses'], source: 'manual', source_id: null, enabled: true }, catalog: { observed_at_ms: 0, models: [{ model_id: 'fixture-model', display_name: 'Fixture model', protocol: 'openai_responses', context_window: null, modalities: ['text'], reasoning_levels: [] }] } } }] };
    let heldProbe, probeBody, cancelBody, probeCount = 0;
    const context = await chromium.launchPersistentContext(profile, { executablePath: '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome', headless: true, viewport: { width: 1440, height: 1000 }, locale: 'en-US', serviceWorkers: 'block', args: ['--disable-background-networking', '--disable-component-update', '--no-first-run', '--disable-sync', '--disable-gpu'] });
    try {
        await context.route('**/*', async route => {
            const request = route.request(), url = new URL(request.url());
            if (url.origin !== origin) { report.blocked.push(url.origin); return route.abort(); }
            const reply = value => route.fulfill({ json: value });
            if (url.pathname === '/api/model-connections' && request.method() === 'GET') return reply(inventory);
            if (url.pathname === '/api/model-connections/probe') { probeCount++; probeBody = request.postDataJSON(); heldProbe = route; return; }
            if (url.pathname === '/api/model-connections/discover/cancel') { cancelBody = request.postDataJSON(); return reply({ cancelled: true }); }
            if (url.pathname.endsWith('/owners')) return reply([]);
            if (request.method() !== 'GET' && request.method() !== 'HEAD') { report.blocked.push(request.method() + ' ' + url.pathname); return route.abort(); }
            return route.continue();
        });
        await context.routeWebSocket('**/*', socket => socket.close());
        const page = await context.newPage();
        page.setDefaultTimeout(10000);
        page.on('pageerror', error => report.errors.push(error.message));
        await page.goto(origin + '/settings', { waitUntil: 'networkidle' });
        const metrics = () => page.evaluate(() => ({ width: innerWidth, ratio: devicePixelRatio }));
        const before = await metrics();
        for (let i = 0; i < 5; i++) await page.keyboard.press('Meta+Equal');
        const after = await metrics();
        report.nativeZoom = { before, after, verified200: Math.abs(after.ratio / before.ratio - 2) < 0.02 && Math.abs(before.width / after.width - 2) < 0.02 };
        await page.keyboard.press('Meta+0');
        await page.goto(origin + '/models?tab=api', { waitUntil: 'networkidle' });
        await page.getByText('Capability evidence & explicit override', { exact: true }).click();
        const send = page.getByRole('button', { name: 'Send one paid-capable text probe', exact: true });
        assert.equal(await send.isDisabled(), true);
        const consent = page.getByRole('checkbox', { name: /I confirm this account/ });
        await consent.check();
        assert.equal(await send.isEnabled(), true);
        await send.click();
        await page.waitForFunction(() => !!document.querySelector('.capability button')?.disabled);
        await page.getByRole('button', { name: 'Cancel probe', exact: true }).click();
        await page.getByText('Cancelled before evidence commit. Provider charges cannot be undone.', { exact: true }).waitFor();
        assert.deepEqual(cancelBody, { request_id: probeBody.request_id, connection_id: 'fixture-connection', expected_revision: 11 });
        assert.equal(probeBody.account_id, 'fixture-account');
        assert.equal(probeBody.account_revision, 7);
        assert.equal(probeBody.credential_binding, 'fixture-binding');
        assert.equal(probeBody.model_id, 'fixture-model');
        assert.equal(probeBody.protocol, 'openai_responses');
        assert.equal(probeBody.max_requests, 1);
        assert.equal(probeBody.acknowledge_possible_charges, true);
        await heldProbe.fulfill({ status: 409, json: { code: 'model_operation_cancelled', error: 'Cancelled fixture' } });
        await consent.waitFor({ state: 'visible' });
        await page.waitForFunction(() => !document.querySelector('.capability input[type=checkbox]')?.disabled);
        assert.equal(await consent.isChecked(), false);
        assert.equal(await send.isDisabled(), true);
        assert.equal(probeCount, 1);
        assert.equal(await page.getByText('Text response verified.', { exact: false }).count(), 0);
        report.finalStatus = await page.locator('.capability [role=status]').textContent();
        report.checks.push('Explicit consent required; exact account/revision/model/protocol binding', 'Cancel binds exact request; completion cannot verify or auto-retry; consent cleared');
        await page.screenshot({ path: path.join(output, 'probe-cancelled.png'), animations: 'disabled' });
        assert.deepEqual(report.blocked, []);
        assert.deepEqual(report.errors, []);
    } finally {
        fs.writeFileSync(path.join(output, 'result.json'), JSON.stringify(report, null, 2));
        console.log(output);
        await context.close();
    }
}
main().catch(error => { console.error(error); process.exitCode = 1; });
