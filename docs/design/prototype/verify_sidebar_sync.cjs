// SPDX-License-Identifier: Apache-2.0
// Local prototype acceptance; external requests are blocked. No product or model calls.
const {chromium} = require('../../../apps/web/playwright_runtime/node_modules/playwright-core');
const env = require('../../../apps/web/playwright_runtime/test_environment.cjs');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const base = process.env.PROTOTYPE_URL || 'http://127.0.0.1:8071';
const out = process.env.PROTOTYPE_OUT || env.artifactPath('nola-sidebar-prototype-20261117');
(async () => {
    fs.mkdirSync(out, {recursive: true});
    const browser = await chromium.launch({...env.browserOptions(), headless: true});
    const receipt = {kind: 'prototype-only', cases: [], screens: [], errors: [], externalRequests: []};
    try {
        for (const width of [1000, 1440]) for (const theme of ['light', 'dark']) {
            const context = await browser.newContext({viewport: {width, height: 960}, colorScheme: theme, reducedMotion: 'no-preference'});
            const page = await context.newPage();
            page.setDefaultTimeout(8000);
            await page.addInitScript(theme => localStorage.setItem('nola-prototype-theme', theme), theme);
            await context.route('**/*', route => new URL(route.request().url()).origin === base ? route.continue() : (receipt.externalRequests.push(route.request().url()), route.abort()));
            page.on('pageerror', error => receipt.errors.push(error.message));
            const click = action => page.locator(`[data-action="${action}"]:visible`).last().click();
            async function shot(name) {
                await page.waitForTimeout(180);
                await page.locator('#toast.visible').waitFor({state: 'hidden'});
                assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), name + ' horizontal overflow');
                const file = `${width}-${theme}-${name}.png`;
                await page.screenshot({path: `${out}/${file}`});
                receipt.screens.push(file);
            }

            await page.goto(base + '/?stream=hold');
            assert.equal(await page.locator('[aria-label="左侧列表类别"]').count(), 0);
            assert.equal(await page.locator('.content-tabs').count(), 0);
            assert.equal(await page.locator('.rail [aria-label="主导航"] button').count(), 4);
            assert.equal(await page.locator('#left-search').count(), 1);
            assert.equal(await page.locator('.rail [data-action="nav:settings"]').count(), 1);
            assert.equal(await page.locator('.rail-history .eh-tree').count(), 0);
            assert.ok((await page.locator('.topbar-title').innerText()).trim().length > 0);

            await click('sn-section:extensions');
            assert.equal(await page.locator('.studio-inspector').count(), 0);
            assert.equal(await page.locator('[data-action^="sn-group:"]').count(), 0);
            assert.equal(await page.locator('#left-search:visible').count(), 1);
            assert.equal(await page.locator('.er-embedded').count(), 1);
            await shot('extensions-experts');
            await click('dk-ext-tab:skills');
            assert.ok((await page.locator('.cap-toolbar').innerText()).includes('这台 Mac'));
            await click('dk-ext-tab:mcp');
            await shot('extensions-mcp');

            await click('sn-section:automations');
            assert.equal(await page.locator('.studio-inspector').count(), 0);
            assert.equal(await page.locator('[aria-label="自动化中心"]').count(), 1);

            await click('sn-section:memory');
            assert.equal(await page.locator('.studio-inspector').count(), 0);
            assert.equal(await page.locator('[data-action="dk-data-pane:memory"]').getAttribute('aria-selected'), 'true');
            for (const pane of ['sources', 'usage', 'memory']) {
                await click('dk-data-pane:' + pane);
                assert.equal(await page.locator(`[data-action="dk-data-pane:${pane}"]`).getAttribute('aria-selected'), 'true');
            }
            await shot('memory-center');

            await click('dk-select:design');
            assert.equal(await page.locator('[data-action^="sn-group:"]').count(), 3);
            assert.equal(await page.locator('.studio-inspector').count(), 1);
            assert.equal(await page.locator('[data-inspector-section]').count(), 0);
            assert.equal(await page.locator('.conversation-controls>button').count(), 3);
            assert.ok(!await page.evaluate(() => document.body.innerText.includes('会话沙盒') || document.body.innerText.includes('执行单元')));

            const owner = await page.evaluate(() => activeId);
            await click('dk-workspace');
            await click('dk-workspace-set:演示项目');
            assert.equal(await page.evaluate(() => activeId), owner);
            assert.ok((await page.locator('[data-action="dk-workspace"]').innerText()).includes('演示项目'));

            if (await page.locator('[data-action="sn-group:任务"]').getAttribute('aria-pressed') !== 'true') await click('sn-group:任务');
            await page.locator('[data-inspector-area="task"]').waitFor();
            assert.equal(await page.locator('[data-action^="eh:toggle:"]').count(), 1);
            await shot('inspector-task');

            await click('sn-group:产物');
            await page.locator('[data-inspector-area="artifacts"]').waitFor();
            assert.ok((await page.locator('.studio-inspector').innerText()).includes('审阅工作区更改'));
            await shot('inspector-artifacts');

            await click('sn-group:上下文');
            await page.locator('[data-inspector-area="context"]').waitFor();
            assert.equal(await page.locator('.inspector-knowledge details[open]').count(), 0);
            const countsText = await page.locator('.knowledge-counts').innerText();
            assert.ok(countsText.includes('条原始消息') && countsText.includes('份摘要'));
            await page.locator('.inspector-knowledge summary').filter({hasText: /^会话 Wiki$/}).click();
            await page.getByLabel('Wiki草稿', {exact: true}).fill('SIDEBAR-DRAFT');
            await click('sn-group:任务');
            await click('sn-group:上下文');
            assert.equal(await page.getByLabel('Wiki草稿', {exact: true}).inputValue(), 'SIDEBAR-DRAFT');
            assert.equal(await page.locator('.inspector-browser-setup').count(), 1);
            await click('dk-browser');
            assert.equal(await page.locator('.inspector-browser-setup').count(), 0);
            await shot('inspector-context');

            const cleanOwner = await page.evaluate(() => conversations.find(c => !c.messages.some(m => m.permission === 'pending')).id);
            await click('dk-select:' + cleanOwner);
            await page.locator('#compose').fill('检查原型的流式状态');
            await page.locator('#composer-form button[type=submit]').click();
            const status = page.locator('.flowing-status').first();
            await status.waitFor();
            assert.equal(await status.locator('.flow-highlight').evaluate(e => getComputedStyle(e).animationName), 'status-flow');
            await shot('processing-flow');
            await page.emulateMedia({reducedMotion: 'reduce'});
            assert.equal(await status.locator('.flow-highlight').evaluate(e => getComputedStyle(e).display), 'none');
            await click('stop');

            receipt.cases.push({width, theme, passed: true});
            await context.close();
        }
        assert.deepEqual(receipt.errors, []);
        assert.deepEqual(receipt.externalRequests, []);
        console.log(`PASS ${receipt.cases.length} sidebar layouts, ${receipt.screens.length} captures`);
    } finally {
        fs.writeFileSync(out + '/receipt.json', JSON.stringify(receipt, null, 2));
        await browser.close();
    }
})().catch(error => {
    console.error(error);
    process.exitCode = 1;
});
