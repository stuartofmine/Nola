const {chromium} = require('playwright-core');
const assert = require('node:assert/strict');
(async () => {
    const browser = await chromium.launch({executablePath: process.env.CHROME_PATH || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome', headless: true});
    try {
        for (const compact of [false, true]) {
            const page = await browser.newPage({viewport: {width: compact ? 390 : 1280, height: 950}, hasTouch: compact});
            page.setDefaultTimeout(10000);
            const errors = [];
            page.on('pageerror', error => errors.push(error.message));
            let memoryEnabled = false, memoryWrites = 0;
            await page.route('**/api/assistant/memory-default', async route => {
                if (route.request().method() === 'PUT') { memoryEnabled = route.request().postDataJSON().enabled; memoryWrites++; }
                await route.fulfill({json: {enabled: memoryEnabled, hostAllowed: true}});
            });
            await page.addInitScript(() => {
                const original = HTMLSelectElement.prototype.showPicker;
                window.pickerCalls = 0;
                HTMLSelectElement.prototype.showPicker = function () { window.pickerCalls++; return original.call(this); };
            });
            await page.goto(`${process.env.UI_FIXTURE_URL || 'http://127.0.0.1:5129'}/review-tests/tests/hit-targets.html`);
            const clickAt = async (selector, x) => {
                const element = page.locator(selector);
                await element.scrollIntoViewIfNeeded();
                const box = await element.boundingBox();
                await page.mouse.click(box.x + x(box.width), box.y + box.height / 2);
            };
            for (const selector of ['#native-details > summary', '.execution-process > summary']) {
                const summary = page.locator(selector);
                for (const target of ['text', 'leading', 'blank']) {
                    if (target === 'text') await summary.locator('span').first().click();
                    else await clickAt(selector, width => target === 'leading' ? 5 : width - 5);
                    assert.equal(await summary.evaluate(e => e.parentElement.open), true, `${selector}: ${target}`);
                    await summary.focus(); await page.keyboard.press('Enter');
                    assert.equal(await summary.evaluate(e => e.parentElement.open), false);
                }
                assert.ok((await summary.boundingBox()).height >= (compact ? 44 : 32));
            }
            for (const selector of ['#select-label > span', '.permission-mode label > span']) {
                const before = await page.evaluate(() => window.pickerCalls);
                await page.locator(selector).click();
                assert.equal(await page.evaluate(() => window.pickerCalls), before + 1, 'exactly one native picker from real text click');
                await page.keyboard.press('Escape');
            }
            // Blank space belongs to the label too; it must not merely focus.
            const beforeBlank = await page.evaluate(() => window.pickerCalls);
            await page.locator('#select-label').click({position: {x: 200, y: 8}});
            assert.equal(await page.evaluate(() => window.pickerCalls), beforeBlank + 1);
            await page.keyboard.press('Escape');
            const beforeNative = await page.evaluate(() => window.pickerCalls);
            await page.locator('#select').click(); await page.keyboard.press('Escape');
            assert.equal(await page.evaluate(() => window.pickerCalls), beforeNative, 'native control must not be forwarded twice');
            await page.locator('#select').focus(); await page.keyboard.press('Space'); await page.keyboard.press('End'); await page.keyboard.press('Enter');
            await page.keyboard.press('Escape');
            assert.equal(await page.locator('#select').evaluate(e => e === document.activeElement), true, 'native keyboard path retains select focus');
            console.log('native picker checks complete');
            await page.locator('#linked-label a').click();
            assert.equal(await page.evaluate(() => window.pickerCalls), beforeNative, 'embedded links keep their own action');
            await page.locator('#check-row span').click(); assert.equal(await page.locator('#check').isChecked(), true);
            await clickAt('#check-row', width => width - 5); assert.equal(await page.locator('#check').isChecked(), false);
            await page.locator('#check').focus(); await page.keyboard.press('Space'); assert.equal(await page.locator('#check').isChecked(), true);
            console.log('checkbox keyboard checks complete');
            const memory = page.locator('[aria-labelledby="memory-default-title"] label');
            await memory.waitFor();
            for (const point of ['text', 'blank', 'checkbox']) {
                const before = memoryWrites;
                const input = memory.locator('input');
                if (point === 'checkbox') await input.click();
                else await clickAt('[aria-labelledby="memory-default-title"] label', width => point === 'text' ? 80 : width - 5);
                await page.waitForFunction(() => !document.querySelector('[aria-labelledby="memory-default-title"] input').disabled);
                assert.equal(memoryWrites, before + 1, `memory ${point} toggles once`);
            }
            assert.ok((await memory.boundingBox()).height >= (compact ? 44 : 32));
            await page.locator('#disable').click();
            const disabledCalls = await page.evaluate(() => window.pickerCalls);
            await clickAt('#select-label > span', () => 10);
            await clickAt('#disabled-fieldset', () => 100);
            await clickAt('#check-row span', () => 10);
            assert.equal(await page.evaluate(() => window.pickerCalls), disabledCalls);
            assert.equal(await page.locator('#check').isChecked(), true);
            // Missing/denied showPicker falls back to native label focus.
            await page.locator('#disable').click();
            await page.evaluate(() => HTMLSelectElement.prototype.showPicker = undefined);
            await page.locator('#select-label > span').click();
            assert.equal(await page.locator('#select').evaluate(e => e === document.activeElement), true);
            assert.deepEqual(errors, []);
            console.log(`PASS ${compact ? 'compact/touch' : 'desktop'}: summary text/icon/blank, select label native picker/keyboard/disabled/link/fallback, configuration checkbox text/blank/control/keyboard and exact single writes`);
            if (process.env.UI_EVIDENCE_DIR) {
                require('node:fs').mkdirSync(process.env.UI_EVIDENCE_DIR, {recursive: true});
                await page.screenshot({path: `${process.env.UI_EVIDENCE_DIR}/hit-targets-${compact ? 'compact' : 'desktop'}.png`, fullPage: true});
            }
            if (compact) {
                await page.goto(`${process.env.UI_FIXTURE_URL || 'http://127.0.0.1:5129'}/chat?conversation=review`);
                await page.locator('#chat-composer').waitFor();
                const summary = page.locator('.composer-options > summary');
                for (const x of [30, 320]) {
                    await clickAt('.composer-options > summary', width => Math.min(x, width - 5));
                    assert.equal(await summary.evaluate(e => e.parentElement.open), true);
                    if (process.env.UI_EVIDENCE_DIR && x === 320) await page.screenshot({path: `${process.env.UI_EVIDENCE_DIR}/chat-compact-options-open.png`});
                    await summary.focus(); await page.keyboard.press('Enter');
                    assert.equal(await summary.evaluate(e => e.parentElement.open), false);
                }
                const undersized = await page.locator('.chat-header button, .chat-header summary').evaluateAll(elements => elements.filter(e => e.getClientRects().length && (e.getBoundingClientRect().height < 44 || e.getBoundingClientRect().width < 44)).map(e => e.outerHTML));
                assert.deepEqual(undersized, []);
                assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
                if (process.env.UI_EVIDENCE_DIR) await page.screenshot({path: `${process.env.UI_EVIDENCE_DIR}/chat-compact-targets.png`});
                console.log('PASS actual compact chat: composer settings text/right whitespace/keyboard; header targets >=44px and no horizontal overflow');
            }
            await page.close();
        }
    } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
