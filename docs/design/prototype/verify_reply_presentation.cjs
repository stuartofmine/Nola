// SPDX-License-Identifier: Apache-2.0
// Browser Use performs input; Playwright only sets up and inspects owned test pages.
const {chromium} = require('../../../apps/web/playwright_runtime/node_modules/playwright-core');
const {BrowserUseExecutor} = require('../../../apps/web/playwright_runtime/browser_use.cjs');
const env = require('../../../apps/web/playwright_runtime/test_environment.cjs');
const fs = require('node:fs'), os = require('node:os'), path = require('node:path'), net = require('node:net'), assert = require('node:assert/strict');
const base = process.env.PROTOTYPE_URL || 'http://127.0.0.1:8071';
const out = env.artifactPath('cross-reference-polish/prototype');
(async () => {
    fs.mkdirSync(out, {recursive:true});
    const socket = net.createServer(); await new Promise(resolve => socket.listen(0, '127.0.0.1', resolve));
    const port = socket.address().port; await new Promise(resolve => socket.close(resolve));
    const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'nola-reply-prototype-'));
    let context, executor;
    const report = {kind:'synthetic-prototype', input:'browser-use', cases:[], errors:[]};
    try {
        context = await chromium.launchPersistentContext(profile, {...env.browserOptions(), headless:true,
            viewport:{width:1000,height:900}, reducedMotion:'reduce', args:[`--remote-debugging-port=${port}`]});
        const page = context.pages()[0]; page.setDefaultTimeout(10000);
        await context.route('**/*', route => new URL(route.request().url()).origin === base ? route.continue() : route.abort());
        page.on('pageerror', error => report.errors.push(error.message));
        executor = await BrowserUseExecutor.attach(`http://127.0.0.1:${port}`, context);
        const click = async locator => {
            const element = await locator.elementHandle(); assert.ok(element);
            try { await executor.perform(page, 'click', {}, element); } finally { await element.dispose(); }
        };
        for (const width of [1000, 390, 320]) {
            await page.setViewportSize({width,height:900});
            await executor.perform(page, 'navigate', {url:base+'/reply-preview.html'});
            await page.locator('.reply-references li').first().waitFor();
            for (const theme of ['light', 'dark']) {
                if (theme === 'dark') await click(page.locator('#theme'));
                assert.equal(await page.locator('.reply-references li').count(), 3);
                assert.equal(await page.locator('td a.inline-reference').count(),2);
                await click(page.locator('[data-stage="reading"]'));
                assert.equal(await page.locator('.reply-process>summary span').first().innerText(), '正在阅读来源');
                await click(page.locator('.reply-process>summary'));
                await page.waitForFunction(() => document.querySelector('.reply-process').open);
                await click(page.locator('[data-stage="waiting"]'));
                assert.equal(await page.locator('.reply-process').getAttribute('open'), '');
                assert.equal(await page.locator('.reply-process>summary span').first().innerText(), '正在处理中');
                await click(page.locator('[data-stage="incomplete"]'));
                assert.equal(await page.locator('.reply-skill>small').innerText(), '未完成');
                await click(page.locator('[data-reference-action]'));
                assert.equal(await page.locator('.reply-references li').count(), 31);
                await click(page.locator('[data-reference-action]'));
                assert.equal(await page.locator('.reply-references li').count(), 3);
                assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
                const color = await page.locator('.reply-references a').first().evaluate(element => getComputedStyle(element).color);
                assert.match(color, /rgb\((\d+), \1, \1\)/, 'Citations use neutral gray');
                await page.screenshot({path:path.join(out, `${width}-${theme}.png`),fullPage:true});
                report.cases.push({width,theme,color,sourceCount:31});
                await click(page.locator('.reply-process>summary'));
            }
        }
        // The same widget is mounted inside both actual conversation prototypes.
        for (const width of [1000,390]) {
            await page.setViewportSize({width,height:900});
            await executor.perform(page, 'navigate', {url:base+'/index.html'});
            const row = page.getByRole('button', {name:/本地模型，怎么选更合适/}).first();
            await row.waitFor(); await click(row);
            await page.locator('.reply-presentation').waitFor();
            assert.equal(await page.locator('.reply-references li').count(),3);
            report.cases.push({width,surface:'conversation'});
        }
        assert.deepEqual(report.errors, []);
        fs.writeFileSync(path.join(out,'receipt.json'),JSON.stringify(report,null,2));
        console.log(`PASS ${report.cases.length} Browser Use prototype cases`);
    } finally {
        if (executor) await executor.close();
        if (context) await context.close();
        fs.rmSync(profile,{recursive:true,force:true});
    }
})().catch(error => { console.error(error); process.exitCode=1; });
