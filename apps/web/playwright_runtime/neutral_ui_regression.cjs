// Isolated actual-route acceptance; start ui_review_fixture.py on 5129 first.
const { chromium } = require('playwright-core');
const assert = require('node:assert/strict');
const fs = require('node:fs');
(async () => {
    const browser = await chromium.launch({...require('./test_environment.cjs').browserOptions(), headless: true});
    const base = process.env.UI_FIXTURE_URL || 'http://127.0.0.1:5129';
    const out = process.env.UI_EVIDENCE_DIR;
    if (out) fs.mkdirSync(out, {recursive: true});
    try {
        for (const colorScheme of ['light', 'dark']) {
            const page = await browser.newPage({viewport: {width: 1440, height: 1000}, colorScheme});
            const errors = [];
            page.on('pageerror', e => errors.push(e.message));
            await page.goto(`${base}/chat?conversation=review`);
            await page.locator('#chat-composer').waitFor();
            assert.equal(await page.locator('.workspace-sidebar').first().evaluate(e => e.contains(document.activeElement)), false, 'inline navigation must not claim initial focus');
            await page.locator('.conversation[aria-current]').waitFor();
            const selected = page.locator('.conversation[aria-current]');
            await selected.hover();
            assert.equal(await selected.evaluate(e => getComputedStyle(e).backgroundColor), 'rgba(0, 0, 0, 0)');
            // Language disclosure is entirely clickable; Escape restores its trigger.
            const language = page.locator('.workspace-sidebar > .action-disclosure');
            await language.locator('summary').click({position: {x: 8, y: 15}});
            await language.locator('select').focus();
            await page.keyboard.press('Escape');
            assert.equal(await language.evaluate(e => e.open), false);
            assert.equal(await language.locator('summary').evaluate(e => e === document.activeElement), true);
            await language.locator('summary').click();
            await page.locator('#chat-composer').focus();
            assert.equal(await language.evaluate(e => e.open), false);
            await page.locator('.execution-process > summary').click();
            assert.equal(await page.locator('.execution-process').evaluate(e => e.open), true);
            if (out) await page.screenshot({path: `${out}/chat-${colorScheme}-expanded.png`});
            for (const width of [320, 390, 768, 1024, 1440]) {
                await page.setViewportSize({width, height: 900});
                await page.evaluate(() => new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r))));
                assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true, `chat overflow ${width}`);
                const composer = await page.locator('.composer').boundingBox();
                assert.ok(composer.x >= 0 && composer.x + composer.width <= width + 1);
                await page.locator('.model-menu summary').click();
                const menu = await page.locator('.model-menu .choices').boundingBox();
                assert.ok(menu.x >= 0 && menu.x + menu.width <= width + 1, `menu overflow ${width}`);
                await page.keyboard.press('Escape');
            }
            await page.route('**/api/chat/conversations', route => route.fulfill({json: {conversations: []}}));
            await page.goto(`${base}/chat`);
            await page.locator('.new-conversation').waitFor();
            assert.equal(await page.locator('.new-conversation').count(), 1);
            await page.locator('.empty-conversation').waitFor();
            assert.equal(await page.locator('#chat-composer').count(), 0);
            await page.unroute('**/api/chat/conversations');
            const parameters = {contextTokens: 65536, temperature: .7, topP: .9, topK: 40, minP: .05, batchSize: 512, threads: 0, gpuLayers: 99, flashAttention: true};
            await page.route('**/api/local-models', route => route.fulfill({json: {binaryAvailable: true, physicalMemoryBytes: 32000000000, recommendedModelId: 'review-local', diskUsageBytes: 5000000000, models: [
                {id: 'review-local', displayName: '隔离本地模型 · 日常写作与代码', installed: true, running: false, selected: true, deviceCompatible: true, configurationRevision: 1, contextWindowTokens: 65536, estimatedRuntimeBytes: 8000000000, recommendedMemoryBytes: 16000000000, downloadBytes: 5000000000, parameters},
                {id: 'review-large', displayName: '长模型名称用于验证窄屏换行与操作按钮布局 Large Reasoning Model', installed: false, running: false, selected: false, deviceCompatible: true, configurationRevision: 1, contextWindowTokens: 131072, parameters}
            ]}}));
            await page.goto(`${base}/models`);
            await page.locator('.model-row').first().waitFor();
            if (out) await page.screenshot({path: `${out}/models-${colorScheme}.png`});
            await page.setViewportSize({width: 390, height: 844});
            await page.evaluate(() => new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r))));
            assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true, 'models overflow');
            if (out) await page.screenshot({path: `${out}/models-${colorScheme}-compact.png`});
            const configure = page.locator('.model-row').first().getByRole('button', {name: /配置|Configure/, exact: true});
            await configure.click();
            const editor = page.locator('dialog:modal').first();
            const context = editor.locator('input[type=number]').first();
            assert.equal(await context.evaluate(e => e === document.activeElement), true, 'model form receives keyboard focus when opened');
            const original = await context.inputValue();
            await context.fill(String(Number(original) + 1));
            await editor.getByRole('button', {name: /取消|Cancel/, exact: true}).click();
            await page.locator('dialog:modal').last().locator('button').first().click();
            assert.equal(await context.inputValue(), String(Number(original) + 1), 'cancel discard preserves model draft');
            await context.fill(original);
            await page.keyboard.press('Escape');
            assert.equal(await page.locator('dialog:modal').count(), 0);
            assert.equal(await configure.evaluate(e => e === document.activeElement), true, 'closing model editor restores configure trigger');
            assert.deepEqual(errors, []);
            await page.close();
        }
        console.log('PASS: light/dark actual chat, border-only selection, disclosure keyboard/outside focus, tool expansion, five widths, model menu bounds, empty chat single entry, models route');
    } finally { await browser.close(); }
})().catch(e => {console.error(e); process.exitCode = 1;});
