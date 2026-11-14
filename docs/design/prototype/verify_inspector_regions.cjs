// SPDX-License-Identifier: Apache-2.0
// Browser Use owns all input; only synthetic prototype state is changed.
const {chromium} = require('../../../apps/web/playwright_runtime/node_modules/playwright-core');
const {BrowserUseExecutor} = require('../../../apps/web/playwright_runtime/browser_use.cjs');
const environment = require('../../../apps/web/playwright_runtime/test_environment.cjs');
const fs = require('node:fs'), os = require('node:os'), path = require('node:path'), net = require('node:net');
const assert = require('node:assert/strict');
const base = process.env.PROTOTYPE_URL || 'http://127.0.0.1:8071';
const output = environment.artifactPath('ios-ui-protocol/prototype');
(async () => {
    fs.mkdirSync(output, {recursive: true});
    const socket = net.createServer(); await new Promise(resolve => socket.listen(0, '127.0.0.1', resolve));
    const port = socket.address().port; await new Promise(resolve => socket.close(resolve));
    const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'nola-inspector-regions-'));
    let context, executor;
    const receipt = {input: 'browser-use', scope: 'synthetic-prototype', cases: [], errors: []};
    try {
        context = await chromium.launchPersistentContext(profile, {...environment.browserOptions(), headless: true,
            viewport: {width: 1440, height: 900}, reducedMotion: 'reduce', args: [`--remote-debugging-port=${port}`]});
        const page = context.pages()[0]; page.setDefaultTimeout(10000);
        await context.route('**/*', route => new URL(route.request().url()).origin === base ? route.continue() : route.abort());
        page.on('pageerror', error => receipt.errors.push(error.message));
        executor = await BrowserUseExecutor.attach(`http://127.0.0.1:${port}`, context);
        async function act(locator, operation = 'click', args = {}) {
            const element = await locator.elementHandle(); assert.ok(element);
            try { await executor.perform(page, operation, args, element); } finally { await element.dispose(); }
        }
        async function area(label, id, count) {
            const control = page.locator(`[data-action="sn-group:${label}"]`);
            if (await control.getAttribute('aria-pressed') !== 'true') await act(control);
            await page.locator(`[data-inspector-area="${id}"]`).waitFor();
            assert.equal(await page.locator('[data-inspector-section]').count(), count);
        }
        for (const scheme of ['light', 'dark']) {
            await page.setViewportSize({width: 1440, height: 900});
            await page.emulateMedia({colorScheme: scheme});
            await page.addInitScript(theme => localStorage.setItem('nola-prototype-theme', theme), scheme);
            await executor.perform(page, 'navigate', {url: base});
            await page.locator('.topbar-left [data-action="new"]').waitFor();
            assert.equal(await page.locator('.topbar-left [data-action="new"]').count(), 1);
            for (const [label, id, count] of [['任务', 'task', 3], ['产物', 'artifacts', 2], ['上下文', 'context', 4]]) {
                await area(label, id, count);
                assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
                await page.screenshot({path: path.join(output, `${scheme}-${id}.png`)});
                receipt.cases.push({scheme, id, count});
            }
            await act(page.locator('.inspector-knowledge summary').filter({hasText: /^Wiki$/}));
            await act(page.getByLabel('Wiki草稿', {exact: true}), 'type_text', {text: '保留独立区域中的未保存草稿'});
            await area('任务', 'task', 3); await area('上下文', 'context', 4);
            assert.equal(await page.getByLabel('Wiki草稿', {exact: true}).inputValue(), '保留独立区域中的未保存草稿');
            const owner = await page.locator('.studio-center').getAttribute('data-context');
            await act(page.locator('.topbar-left [data-action="new"]'));
            assert.notEqual(await page.locator('.studio-center').getAttribute('data-context'), owner);
            receipt.cases.push({scheme, flow: 'retained-wiki-draft-and-new-conversation'});
        }
        for (const width of [320, 390]) {
            await page.setViewportSize({width, height: 844});
            await executor.perform(page, 'navigate', {url: base});
            await act(page.locator('[data-action="im:nav:information"]'));
            const layout = await page.evaluate(() => ({width: innerWidth, mobile: matchMedia('(max-width:700px)').matches,
                padding: getComputedStyle(document.querySelector('.im-screen')).paddingBottom,
                header: getComputedStyle(document.querySelector('.im-header')).display,
                sheets: Array.from(document.styleSheets).map(sheet => {
                    // Browser extensions/runtime overlays can own opaque sheets.
                    // This is diagnostic metadata; keep the layout assertions below exact.
                    let rules; try { rules = sheet.cssRules.length; } catch { rules = 'opaque'; }
                    return {href: sheet.href, rules};
                })}));
            assert.equal(layout.width, width, JSON.stringify(layout));
            assert.equal(layout.header, 'flex', JSON.stringify(layout));
            assert.ok(await page.locator('.im-screen [data-action^="im:chat:"]').count() > 0);
            assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
            await page.screenshot({path: path.join(output, `${width}-library.png`)});
            await act(page.locator('[data-action="im:nav:activity"]'));
            assert.equal(await page.locator('.im-screen [data-action="im:nav:plans"]').count(), 1);
            await page.screenshot({path: path.join(output, `${width}-activity.png`)});
            receipt.cases.push({width, flow: 'library-recents-and-plans-entry'});
        }
        assert.deepEqual(receipt.errors, []);
        fs.writeFileSync(path.join(output, 'receipt.json'), JSON.stringify(receipt, null, 2));
        console.log(`PASS ${receipt.cases.length} Browser Use cases`);
    } finally {
        if (executor) await executor.close();
        if (context) await context.close();
        fs.rmSync(profile, {recursive: true, force: true});
    }
})().catch(error => { console.error(error); process.exitCode = 1; });
