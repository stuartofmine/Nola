// Real production DOM. Mutation requests are intercepted; this never invokes a model.
const { chromium } = require('playwright-core');
const fs = require('fs');
const assert = require('node:assert/strict');
const cid = process.env.NOLA_A2UI_CONVERSATION_ID;
const client = process.env.NOLA_A2UI_CLIENT_ID;
assert.ok(cid && client);
(async () => {
    const state = JSON.parse(fs.readFileSync('/tmp/nola-a2ui-real-host-state.json'));
    assert.equal(new URL(state.url).hostname, '127.0.0.1');
    const browser = await chromium.launch({ executablePath: '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome', headless: true });
    const results = [];
    try {
        for (let i = 0; i < 20; i++) {
            const mode = i < 8 ? 'mouse' : i < 14 ? 'touch' : 'keyboard';
            const context = await browser.newContext({
                viewport: mode === 'mouse' ? { width: 1280, height: 900 } : { width: 390, height: 844 },
                hasTouch: mode === 'touch',
            });
            const page = await context.newPage();
            await page.addInitScript(clientId => {
                sessionStorage.setItem('nola.web.chat.client', clientId);
                window.clicks = [];
                for (const type of ['pointerdown', 'pointerup', 'click', 'focusout']) {
                    document.addEventListener(type, event => window.clicks.push({
                        type, target: event.target.outerHTML.slice(0, 220), x: event.clientX, y: event.clientY,
                    }), true);
                }
            }, client);
            let actions = 0;
            await page.route('**/ui-interactions', route => {
                actions++;
                return route.fulfill({ status: 503, json: { error: 'isolated observation' } });
            });
            await page.route('**/api/chat/agent/v2/stream', route => route.abort());
            await page.goto(state.url, { waitUntil: 'domcontentloaded' });
            await page.goto(`${new URL(state.url).origin}/chat?conversation=${encodeURIComponent(cid)}`, { waitUntil: 'domcontentloaded' });
            const surface = page.locator('.surface-reference').first();
            await surface.locator('input:not([disabled])').first().fill(`isolated click ${i}`);
            const button = surface.locator('button.a2ui-button:not([disabled])').first();
            const before = await button.boundingBox();
            if (mode === 'touch') await button.tap();
            else if (mode === 'keyboard') { await button.focus(); await page.keyboard.press('Enter'); }
            else if (i === 1) {
                await button.scrollIntoViewIfNeeded();
                const bounds = await button.boundingBox();
                await page.mouse.move(bounds.x + bounds.width / 2, bounds.y + bounds.height / 2);
                await page.mouse.down();
                await page.waitForTimeout(300); // Deliberate long press, not a readiness workaround.
                await page.mouse.up();
            } else await button.click();
            await page.waitForFunction(() => document.querySelector('.nola-generative-surface [role=alert]'), {}, { timeout: 3000 }).catch(() => {});
            const result = { i, mode, actions, before,
                after: await surface.locator('button.a2ui-button').first().boundingBox(),
                events: await page.evaluate(() => window.clicks), text: await surface.innerText() };
            results.push(result);
            fs.writeFileSync('/tmp/nola-a2ui-cold-click-result.json', JSON.stringify(results, null, 2));
            console.log(JSON.stringify({ i, mode, actions }));
            assert.equal(actions, 1, 'one actual submit request per activation');
            // Ordinary user scroll still exposes navigation, which returns to the latest content.
            if (i === 0) {
                const transcript = page.locator('.transcript');
                await transcript.hover();
                await page.mouse.wheel(0, -10000);
                await page.locator('button.latest').waitFor({ state: 'visible' });
                await page.locator('button.latest').click();
                await page.waitForFunction(() => {
                    const view = document.querySelector('.transcript');
                    return view.scrollHeight - view.clientHeight - view.scrollTop < 2;
                });
            }
            await context.close();
        }
    } finally { await browser.close(); }
})().catch(error => { console.error(error.message); process.exitCode = 1; });
