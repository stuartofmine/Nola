// Hold the actual session response until the user attempts to start a chat.
const { chromium } = require('playwright-core');
const assert = require('node:assert/strict');
const { browserOptions, isolatedHostOrigin } = require('./test_environment.cjs');

async function main() {
    const origin = isolatedHostOrigin();
    const browser = await chromium.launch({...browserOptions(), headless: true});
    let release;
    const gate = new Promise(resolve => { release = resolve; });
    try {
        const page = await browser.newPage({locale: 'en-US', viewport: {width: 1440, height: 900}});
        page.setDefaultTimeout(12000);
        const createRequests = [];
        const creation = request => new URL(request.url()).pathname === '/api/chat/conversations' && request.method() === 'POST';
        page.on('request', request => { if (creation(request)) createRequests.push(request); });
        await page.route('**/api/web/session', async route => { await gate; await route.continue(); }, {times: 1});
        await page.goto(new URL('/chat', origin).href, {waitUntil: 'domcontentloaded'});
        const button = page.getByRole('button', {name: 'New conversation', exact: true});
        await button.waitFor();
        assert.ok(await button.isDisabled());
        await button.dispatchEvent('click');
        assert.equal(createRequests.length, 0);
        release();
        async function create() {
            const [response] = await Promise.all([
                page.waitForResponse(response => creation(response.request())),
                button.click(),
            ]);
            assert.ok(response.ok());
            const {conversationId} = await response.json();
            assert.ok(typeof conversationId === 'string' && conversationId.length > 0);
            await page.waitForURL(url => url.searchParams.get('conversation') === conversationId);
            return conversationId;
        }
        const first = await create();
        const name = 'Navigation audit ' + first;
        await page.getByRole('tab', {name: 'New conversation', exact: true}).dblclick();
        const title = page.getByRole('textbox', {name: 'Conversation title'});
        await title.fill(name);
        await title.press('Enter');
        await page.getByRole('tab', {name, exact: true}).waitFor();
        const composer = page.locator('#chat-composer');
        const draft = 'An unsent navigation draft';
        await composer.fill(draft);
        const second = await create();
        assert.notEqual(second, first);
        assert.equal(await composer.inputValue(), '');
        await page.locator('.conversation').filter({hasText: name}).click();
        await page.waitForURL(url => url.searchParams.get('conversation') === first);
        await page.waitForFunction(value => document.querySelector('#chat-composer')?.value === value, draft);
        assert.equal(createRequests.length, 2);
        console.log('PASS: startup gated, two distinct creations, selected original row restores draft');
    } finally {
        release();
        await browser.close();
    }
}

main().catch(error => { console.error(error); process.exitCode = 1; });
