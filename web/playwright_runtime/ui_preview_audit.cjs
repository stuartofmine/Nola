// Isolated, real-browser UI acceptance. Never use a user Chrome profile or live DB.
const { chromium } = require('playwright-core');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { verifyPreviewOwner } = require('./ui_preview_owner.cjs');

async function main() {
    const origin = 'http://127.0.0.1:5001';
    const database = process.env.NOLA_UI_PREVIEW_DB;
    assert.match(database || '', /^\/tmp\/nola-web-preview\.[^/]+\/state\.db$/);
    assert.ok(fs.existsSync(database), 'An explicitly confirmed temporary preview database is required');
    const owner = verifyPreviewOwner(database);
    const output = fs.mkdtempSync(path.join(os.tmpdir(), 'nola-web-ui-acceptance.'));
    const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'nola-web-ui-chrome.'));
    const blocked = [], errors = [], checks = [], visualIssues = [];
    const simulateZoom = process.env.NOLA_UI_CSS_ZOOM === '1';
    const context = await chromium.launchPersistentContext(profile, {
        executablePath: '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
        headless: true, viewport: { width: 1440, height: 1000 }, locale: 'en-US',
        colorScheme: 'light', serviceWorkers: 'block',
        args: ['--disable-background-networking', '--disable-component-update', '--no-first-run', '--disable-sync', '--disable-gpu'],
    });
    let failed;
    try {
        await context.route('**/*', async route => {
            const request = route.request(), url = new URL(request.url());
            const allowedPosts = ['/api/chat/conversations', '/api/chat/agent/v2/observe',
                '/api/model-connections/kimi-login/owners', '/api/model-connections/opencode-login/owners'];
            const allowed = url.origin === origin && (['GET', 'HEAD'].includes(request.method())
                || request.method() === 'POST' && allowedPosts.includes(url.pathname));
            if (!allowed) { blocked.push(`${request.method()} ${url.origin}${url.pathname}`); return route.abort(); }
            if (request.method() === 'POST') {
                try {
                    const current = verifyPreviewOwner(database);
                    assert.equal(current.pid, owner.pid);
                } catch {
                    blocked.push('preview_listener_owner_changed');
                    return route.abort();
                }
            }
            return route.continue();
        });
        await context.routeWebSocket('**/*', socket => { blocked.push('WebSocket blocked'); socket.close(); });
        const page = await context.newPage();
        page.on('pageerror', error => errors.push(error.message));
        page.setDefaultTimeout(15000);
        async function capture(name) {
            await page.screenshot({ path: path.join(output, `${name}.png`), fullPage: false, animations: 'disabled' });
            const overflow = await page.evaluate(() => ({ viewport: innerWidth, layout: document.documentElement.clientWidth, width: document.documentElement.scrollWidth }));
            assert.ok(overflow.width <= overflow.layout + 1, `${name}: horizontal overflow ${JSON.stringify(overflow)}`);
            const appearance = await page.evaluate(() => {
                const button = document.querySelector('button');
                return { zoom: getComputedStyle(document.documentElement).zoom,
                    background: getComputedStyle(document.documentElement).backgroundColor,
                    buttonBackground: button ? getComputedStyle(button).backgroundColor : null,
                    headings: [...document.querySelectorAll('h1')].map(element => ({ text: element.textContent, top: element.getBoundingClientRect().top })),
                    mainCount: document.querySelectorAll('#page-main').length,
                    chatWidth: document.querySelector('.chat-main')?.getBoundingClientRect().width ?? null,
                    tabs: [...document.querySelectorAll('button[role="tab"]')].map(element => ({
                        text: element.textContent, color: getComputedStyle(element).color,
                        background: getComputedStyle(element).backgroundColor,
                        selected: element.getAttribute('aria-selected'),
                    })),
                };
            });
            assert.equal(appearance.headings.length, 1, `${name}: duplicate page headings`);
            assert.equal(appearance.mainCount, 1, `${name}: duplicate main content`);
            if (appearance.chatWidth !== null && appearance.chatWidth < Math.min(400, overflow.viewport)) {
                visualIssues.push(`${name}: reading column is narrower than the available-width minimum; CSS zoom does not change browser breakpoints`);
            }
            checks.push({ name, ...overflow, ...appearance });
        }
        await page.goto(`${origin}/chat`, { waitUntil: 'networkidle' });
        await page.locator('.empty-conversation button, #chat-composer').first().waitFor();
        if (await page.locator('.empty-conversation button').count()) {
            assert.equal(await page.locator('#chat-composer').count(), 0);
            assert.equal(await page.locator('#chat-details').count(), 0);
            await capture('chat-empty-wide');
        }
        async function create() {
            const receipt = page.waitForResponse(response => response.url() === `${origin}/api/chat/conversations` && response.request().method() === 'POST');
            await page.getByRole('button', { name: 'New conversation', exact: true }).last().click();
            const response = await receipt;
            assert.ok(response.ok(), 'Fixture conversation must be confirmed');
            await page.locator('#chat-composer').waitFor();
            await page.waitForURL(/conversation=/);
            return page.url();
        }
        const reuse = process.env.NOLA_UI_REUSE_FIXTURES === '1';
        const first = reuse ? page.url() : await create();
        await page.locator('#chat-composer').fill('Draft A — 未发送的独立草稿');
        if (reuse) {
            await page.locator('.conversation-list button.conversation:not([aria-current])').first().click();
            await page.waitForURL(url => url.href !== first);
        }
        const second = reuse ? page.url() : await create();
        assert.notEqual(first, second);
        assert.equal(await page.locator('#chat-composer').inputValue(), '');
        await page.locator('#chat-composer').fill('Draft B — separate, never submitted');
        const rows = page.locator('.conversation-list button.conversation');
        await rows.first().waitFor();
        await page.locator('.conversation-list button.conversation:not([aria-current])').first().click();
        await page.waitForURL(first);
        assert.equal(await page.locator('#chat-composer').inputValue(), 'Draft A — 未发送的独立草稿');
        await page.locator('.conversation-list button.conversation:not([aria-current])').first().click();
        await page.waitForURL(second);
        assert.equal(await page.locator('#chat-composer').inputValue(), 'Draft B — separate, never submitted');
        checks.push({ name: 'two-conversation-draft-isolation', passed: true });
        await capture('chat-draft-wide');
        const reached = new Set();
        await page.locator('.skip-content').focus();
        for (let index = 0; index < 90 && reached.size < 3; index++) {
            await page.keyboard.press('Tab');
            const target = await page.evaluate(() => {
                const element = document.activeElement;
                if (element?.id === 'chat-composer') return 'composer';
                if (element?.matches('button.conversation')) return 'conversation';
                if (element?.matches('button') && element.textContent === 'New conversation') return 'new-conversation';
                return null;
            });
            if (target) reached.add(target);
        }
        assert.equal(reached.size, 3, 'Real Tab traversal must reach create, conversation and composer');
        checks.push({ name: 'keyboard-tab', reached: [...reached] });
        if (simulateZoom) {
            await page.evaluate(() => { document.documentElement.style.zoom = '2'; });
            await capture('chat-css-zoom-200');
            await page.evaluate(() => { document.documentElement.style.zoom = ''; });
        }
        await page.setViewportSize({ width: 320, height: 740 });
        await capture('chat-draft-narrow');
        await page.setViewportSize({ width: 1440, height: 1000 });
        for (const [name, target] of [['settings', '/settings'], ['models', '/models?tab=api']]) {
            // Navigate through the app's own same-document navigation to preserve drafts.
            await page.locator(`a[href="${target.split('?')[0]}"]`).first().click();
            if (name === 'models') await page.getByRole('tab', { name: /API/ }).click();
            await page.waitForLoadState('networkidle');
            await capture(`${name}-wide`);
            if (simulateZoom) {
                await page.evaluate(() => { document.documentElement.style.zoom = '2'; });
                await capture(`${name}-css-zoom-200`);
                await page.evaluate(() => { document.documentElement.style.zoom = ''; });
            }
            await page.emulateMedia({ colorScheme: 'dark' });
            await page.setViewportSize({ width: 320, height: 740 });
            await capture(`${name}-narrow-dark`);
            await page.setViewportSize({ width: 1440, height: 1000 });
            await page.emulateMedia({ colorScheme: 'light' });
        }
        assert.deepEqual(blocked, [], 'No forbidden network operation may be attempted');
        assert.deepEqual(errors, [], 'No browser runtime errors');
        assert.deepEqual(visualIssues, [], 'A non-overflowing page can still have an unreadable reading column');
    } catch (error) { failed = error; }
    finally {
        fs.writeFileSync(path.join(output, 'result.json'), JSON.stringify({ database, output, profile, simulateZoom, checks, blocked, errors, visualIssues, failure: failed?.message ?? null }, null, 2));
        await context.close();
        console.log(output);
    }
    if (failed) throw failed;
}
main().catch(error => { console.error(error); process.exitCode = 1; });
