// Exercise the mobile entry point against a disposable, empty-model Rust Host.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { chromium } = require('playwright-core');
const { artifactPath, browserOptions, isolatedHostOrigin } = require('./test_environment.cjs');

async function main() {
    const origin = isolatedHostOrigin();
    const out = artifactPath(process.env.NOLA_NARROW_AUDIT_FOLDER || 'nola-web-narrow-navigation');
    fs.mkdirSync(out, {recursive: true});

    const browser = await chromium.launch({...browserOptions(), headless: true});
    const page = await browser.newPage({locale: 'en-US', viewport: {width: 390, height: 844}});
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    const composer = page.locator('#chat-composer');
    const shot = async name => {
        await page.evaluate(async () => {
            await document.fonts.ready;
            await new Promise(requestAnimationFrame);
            await Promise.allSettled(document.getAnimations().filter(animation =>
                animation.effect?.getComputedTiming().endTime !== Infinity).map(animation => animation.finished));
        });
        await page.screenshot({path: path.join(out, name + '.png'), animations: 'disabled'});
    };
    async function create() {
        const [response] = await Promise.all([
            page.waitForResponse(response => new URL(response.url()).pathname === '/api/chat/conversations'
                && response.request().method() === 'POST'),
            page.getByRole('button', {name: 'New conversation', exact: true}).click(),
        ]);
        assert.ok(response.ok());
        const {conversationId} = await response.json();
        assert.ok(typeof conversationId === 'string' && conversationId.length > 0);
        await page.waitForURL(url => url.pathname === '/chat' && url.searchParams.get('conversation') === conversationId);
        await composer.waitFor();
        return conversationId;
    }
    try {
        await page.goto(new URL('/chat', origin).href);
        await page.waitForURL(url => url.pathname === '/conversations');
        const first = await create();
        const draft = 'Keep my draft while I choose a model';
        await composer.fill(draft);
        await page.getByRole('button', {name: 'Send', exact: true}).click();
        const panel = page.getByRole('dialog', {name: 'Context'});
        const add = panel.getByRole('link', {name: 'Add a chat model'});
        await add.waitFor();
        await add.click({trial: true});
        assert.ok(await panel.getByRole('combobox', {name: 'Model', exact: true}).isDisabled());
        await shot('01-empty-catalog-recovery');
        await add.click();
        await page.getByRole('tab', {name: 'API', exact: false}).first().waitFor();
        await page.waitForURL(url => url.pathname === '/models');
        await shot('02-model-configuration');
        await page.goBack();
        await page.waitForURL(url => url.searchParams.get('conversation') === first);
        await page.waitForFunction(value => document.querySelector('#chat-composer')?.value === value, draft);
        await shot('03-restored-draft');
        await page.getByRole('button', {name: 'Workspace', exact: true}).click();
        await page.waitForURL(url => url.pathname === '/conversations');
        const second = await create();
        assert.notEqual(second, first);
        assert.equal(await composer.inputValue(), '');
        assert.deepEqual(errors, []);
        await shot('04-second-conversation');
        console.log('PASS: distinct Host conversations, actionable empty catalog, back restores original draft');
    } catch (error) {
        await shot('failure');
        fs.writeFileSync(path.join(out, 'failure.txt'), await page.locator('body').ariaSnapshot());
        throw error;
    } finally {
        await browser.close();
    }
}

main().catch(error => { console.error(error); process.exitCode = 1; });
