// SPDX-License-Identifier: Apache-2.0
// Real browser history, focus and geometry with a deliberately suspended Host read.
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const {chromium} = require('playwright-core');
const {artifactPath, browserOptions} = require('./test_environment.cjs');

const origin = process.env.UI_FIXTURE_URL || 'http://127.0.0.1:5139';
const evidence = process.env.UI_EVIDENCE_DIR || artifactPath('web-information-navigation');
const fixtures = {
    '/api/data-sources': {sources: [{source: {id: 'journal', name: 'Research notes', kind: 'localFile',
        location: 'notes.json', accessPolicy: 'conversationOptIn', validationContract: null}}]},
    '/api/data-sources/usage': {usageRecords: [{id: 'read-1', sourceId: 'journal', sourceTitle: 'Research notes',
        consumer: 'Research', purpose: 'Read the summary', usedAt: '2026-11-21T10:00:00Z'}]},
    '/api/memory': {type: 'memory_records', limit: 2000, records: [{id: 'one', scope: 'global', kind: 'preference',
        content: 'Use compact tables', summary: null, tags: [], source: 'user', confidence: 1, supersedes: [],
        importance: 0.5, sensitivity: null, expires_at: null, created_at: '2026-11-21', updated_at: '2026-11-21'}]},
    '/api/chat/conversations': {conversations: []},
    '/api/activity': {entries: [], next_cursor: null},
};

async function verifyRefresh(page, width, name, endpoint, panel) {
    const button = panel.getByRole('button', {name: 'Refresh', exact: true});
    const row = panel.locator('article').first();
    await row.waitFor();
    await button.scrollIntoViewIfNeeded();
    const before = await row.boundingBox();
    let resume, count = 0;
    const response = new Promise(resolve => resume = resolve);
    const url = origin + endpoint;
    const hold = async route => { count++; await response; await route.fulfill({json: fixtures[endpoint]}); };
    await page.route(url, hold);
    try {
        await button.focus();
        await Promise.all([page.waitForRequest(request => request.url() === url), button.press('Enter')]);
        await page.waitForFunction(node => node.getAttribute('aria-busy') === 'true', await panel.elementHandle());
        await page.screenshot({path: path.join(evidence, `${name}-${width}-refreshing.png`)});
        const during = await row.boundingBox();
        assert.ok(Math.abs(before.y - during.y) < 1, `${name} row moved ${during.y - before.y}px during refresh`);
        assert.equal(await button.evaluate(node => node === document.activeElement), true, `${name} dropped keyboard focus`);
        await page.keyboard.press('Enter');
    } finally { resume(); }
    await page.waitForFunction(node => node.getAttribute('aria-busy') === 'false', await panel.elementHandle());
    await page.unroute(url, hold);
    assert.equal(count, 1, 'Repeated activation must share the pending read');
    const after = await row.boundingBox();
    assert.ok(Math.abs(before.y - after.y) < 1);
}

async function verifyColdDetail(browser, pathName, moduleName) {
    const page = await browser.newPage({viewport: {width: 390, height: 740}});
    let resume;
    const blocked = new Promise(resolve => resume = resolve);
    const pattern = `**/ui/assets/${moduleName}-*.js`;
    await page.route(pattern, async route => { await blocked; await route.continue(); });
    try {
        await page.goto(origin + pathName, {waitUntil: 'domcontentloaded'});
        await page.locator('#page-main > [role=status]').waitFor({timeout: 8000});
        const main = page.locator('#page-main');
        const before = await main.boundingBox();
        resume();
        await main.locator('h1').waitFor();
        assert.equal((await main.boundingBox()).y, before.y, 'Cold detail must reserve its final header while the module loads');
    } finally { resume(); await page.close(); }
}

async function verify(width, browser) {
    const page = await browser.newPage({viewport: {width, height: 740}, locale: 'en-US', colorScheme: width > 700 ? 'dark' : 'light'});
    page.setDefaultTimeout(8000);
    try {
        await page.route('**/api/**', route => {
            const payload = fixtures[new URL(route.request().url()).pathname];
            return route.fulfill(payload ? {json: payload} : {status: 503, json: {error: 'isolated fixture'}});
        });
        const mobile = width <= 700;
        await page.goto(origin + (mobile ? '/activity' : '/data-sources'));
        const overview = page.locator('.information-navigation');
        if (mobile) {
            for (const root of ['/conversations', '/activity', '/settings', '/data-sources']) {
                await page.locator(`.mobile-primary-navigation [href="${root}"]`).click();
                await page.locator(root === '/conversations' ? '.conversation-list-page' : 'h1').first().waitFor();
                await page.waitForURL(url => url.pathname === root);
                await page.evaluate(() => document.fonts.ready);
                assert.equal(await page.evaluate(() => document.documentElement.scrollHeight), 740,
                    `${root} should fill the viewport without empty space behind the tab bar`);
            }
            await overview.waitFor();
        }
        for (const [section, endpoint, selector] of [
            ['sources', '/api/data-sources', '[id$="data-panel-sources"]'],
            ['memory', '/api/memory', '.memory-center'],
            [mobile ? 'activity' : 'usage', '/api/data-sources/usage', '[id$="data-panel-usage"]'],
        ]) {
            const link = overview.locator(`a[href$="tab=${section}"]`);
            if (mobile) await link.click();
            else await page.locator(`[role="tab"][id$="data-tab-${section}"]`).click();
            const panel = page.locator(selector + ':visible');
            if (mobile) assert.ok(await page.locator('h1').evaluate(node => node === document.activeElement), 'Drill-down announces its heading');
            await verifyRefresh(page, width, section, endpoint, panel);
            if (mobile && section === 'sources') {
                await page.locator('.page-actions .primary').click();
                const dialog = page.locator('dialog:modal');
                await dialog.locator('input').first().fill('Unsubmitted source');
                await page.goBack();
                await page.waitForURL(url => url.searchParams.get('tab') === 'sources');
                await page.getByRole('dialog', {name: 'Discard unsaved changes?'}).getByRole('button', {name: 'Cancel', exact: true}).click();
                assert.equal(await dialog.locator('input').first().inputValue(), 'Unsubmitted source');
                await dialog.locator('input').first().fill('');
                await page.keyboard.press('Escape');
                await dialog.waitFor({state: 'hidden'});
            }
            if (mobile) {
                if (section === 'memory') {
                    await page.setViewportSize({width: 950, height: 740});
                    await page.goBack();
                    await page.locator('[role=tab][id$=data-tab-sources][aria-selected=true]').waitFor();
                    assert.equal(new URL(page.url()).searchParams.has('tab'), false);
                    await page.goForward();
                    await page.locator('[role=tab][id$=data-tab-memory][aria-selected=true]').waitFor();
                    await page.setViewportSize({width, height: 740});
                }
                await page.goBack();
                await overview.waitFor();
                assert.equal(await link.evaluate(node => node === document.activeElement), true, 'Back restores directory link focus');
                await page.goForward();
                await panel.waitFor();
                await page.locator('.information-back button').click();
                await overview.waitFor();
                assert.equal(await link.evaluate(node => node === document.activeElement), true, 'Toolbar Back restores directory link focus');
            }
        }
    } catch (error) {
        await page.screenshot({path: path.join(evidence, `failure-${width}.png`)});
        throw error;
    } finally { await page.close(); }
}

(async () => {
    await fs.mkdir(evidence, {recursive: true});
    const browser = await chromium.launch(browserOptions());
    try {
        await verifyColdDetail(browser, '/data-sources?tab=sources', 'DataSources');
        await verifyColdDetail(browser, '/settings?section=general', 'Settings');
        await verifyColdDetail(browser, '/data-sources?tab=usage', 'DataSources');
        for (const width of [390, 1440]) await verify(width, browser);
    }
    finally { await browser.close(); }
    console.log('Passed compact and desktop information refresh geometry, keyboard focus and compact back/forward traversal.');
})().catch(error => { console.error(error); process.exitCode = 1; });
