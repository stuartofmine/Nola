const {chromium} = require('playwright-core');
const assert = require('node:assert/strict');
const fs = require('node:fs');
(async () => {
    const browser = await chromium.launch({executablePath: process.env.CHROME_PATH || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome', headless: true});
    try {
        for (const width of [320, 390, 1280]) {
            const page = await browser.newPage({viewport: {width, height: 844}, colorScheme: width === 390 ? 'dark' : 'light'});
            const errors = [], bodies = [];
            page.on('pageerror', e => errors.push(e.message));
            await page.route('**/api/agent/host-permission/decision', async route => {
                const body = route.request().postDataJSON(); bodies.push(body);
                await route.fulfill(bodies.length === 1 ? {status: 503, json: {error: 'fixture_failure'}} : {json: {accepted: true, nonce: body.nonce}});
            });
            await page.goto(`${process.env.UI_FIXTURE_URL || 'http://127.0.0.1:5129'}/review-tests/tests/permission.html`);
            const panel = page.locator('.permission-panel');
            await panel.waitFor();
            const actions = panel.locator('.actions button');
            assert.equal(await actions.count(), 4);
            assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
            const bounds = await panel.boundingBox();
            for (const button of await actions.all()) {
                const box = await button.boundingBox();
                assert.ok(box.y >= bounds.y && box.y + box.height <= bounds.y + bounds.height + 1, 'decision stays inside panel without scrolling details');
                assert.ok(box.height >= (width <= 760 ? 44 : 32));
            }
            const guidance = panel.locator('.guidance');
            assert.equal(await guidance.evaluate(e => e.open), false);
            await guidance.locator('summary').click();
            await guidance.locator('textarea').fill('Keep this draft');
            await guidance.locator('summary').click();
            await guidance.locator('summary').click();
            assert.equal(await guidance.locator('textarea').inputValue(), 'Keep this draft');
            await page.locator('#next').click();
            assert.equal(await guidance.evaluate(e => e.open), false, 'new request resets the previous guidance');
            await guidance.locator('summary').click();
            assert.equal(await guidance.locator('textarea').inputValue(), '');
            await guidance.locator('summary').click();
            await page.locator('#missing').click();
            assert.equal(await actions.nth(2).isDisabled(), true);
            assert.equal(await actions.nth(3).isDisabled(), true);
            assert.equal(await actions.nth(0).isEnabled(), true, 'missing safe details must not prevent denial');
            await page.locator('#missing').click();
            await page.locator('#expire').click();
            for (const button of await actions.all()) assert.equal(await button.isDisabled(), true);
            await panel.getByRole('button', {name: /刷新请求状态|Refresh request status/}).click();
            assert.equal(await page.locator('#refresh-count').textContent(), '1');
            await page.locator('#expire').click();
            await actions.nth(3).waitFor({state: 'visible'});
            assert.equal(await actions.nth(3).isEnabled(), true);
            assert.ok((await panel.locator('pre').boundingBox()).height > 40, 'safe details must not collapse to a zero-height grid row');
            assert.equal(await panel.locator('.permission-content').evaluate(e => e.scrollTop), 0, 'new challenge starts at the safe details');
            await page.waitForTimeout(250); // Capture the settled control transition.
            if (process.env.UI_EVIDENCE_DIR) {
                fs.mkdirSync(process.env.UI_EVIDENCE_DIR, {recursive:true});
                await page.screenshot({path: `${process.env.UI_EVIDENCE_DIR}/permission-${width}.png`});
            }
            await actions.nth(3).click();
            await panel.getByRole('button', {name: /重试相同决定|Retry same decision/}).waitFor();
            assert.equal(bodies[0].resolution.decision, 'allow_once');
            await panel.getByRole('button', {name: /重试相同决定|Retry same decision/}).click();
            await page.waitForFunction(() => document.querySelector('.permission-panel [role=status]').textContent.match(/Host.*(接收|received)/));
            assert.deepEqual(bodies[1], bodies[0], 'uncertain transport retries the frozen decision, not the deny placeholder');
            assert.equal(bodies.length, 2);
            assert.deepEqual(errors, []);
            await page.close();
        }
        console.log('PASS: 320/390/1280 approval layout, safe-details guard, denial availability, expired refresh, guidance preservation/reset, unknown receipt and exact retry');
    } finally { await browser.close(); }
})().catch(e => {console.error(e); process.exitCode=1;});
