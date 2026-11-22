// SPDX-License-Identifier: Apache-2.0
// Use a disposable Host with a saved conversation; cancelling does not change its title.
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const { chromium } = require('playwright-core');
const { artifactPath, browserOptions, isolatedHostOrigin } = require('./test_environment.cjs');

(async () => {
    const origin = isolatedHostOrigin();
    const out = process.env.UI_EVIDENCE_DIR || artifactPath('conversation-rename-hit-target');
    await fs.mkdir(out, {recursive:true});
    const browser = await chromium.launch({...browserOptions(), headless:true});
    try {
        for (const {width, entry} of [{width:320, entry:'conversations'}, {width:390, entry:'conversations'}, {width:390, entry:'chat'}]) {
            const page = await browser.newPage({viewport:{width, height:844}, locale:'en', hasTouch:true});
            await page.goto(new URL('/' + entry, origin).href);
            const row = page.locator('.page-list .conversation-row').first();
            const summary = row.locator('.row-actions > summary');
            await summary.click();
            await row.getByRole('button', {name:'Rename', exact:true}).click();
            const form = row.locator('.rename-form');
            const cancel = form.getByRole('button', {name:'Cancel', exact:true});
            await cancel.scrollIntoViewIfNeeded();
            assert.equal(await cancel.evaluate(button => {
                const box = button.getBoundingClientRect();
                return button.contains(document.elementFromPoint(box.x + box.width / 2, box.y + box.height / 2));
            }), true, 'the row actions menu must not cover Cancel');
            await page.screenshot({path:path.join(out, entry+'-'+width+'-editor.png')});
            await cancel.tap();
            await form.waitFor({state:'hidden'});
            assert.equal(await summary.evaluate(node => node === document.activeElement), true);
            await page.screenshot({path:path.join(out, entry+'-'+width+'-cancelled.png')});
            await page.close();
        }
        console.log('PASS: touch Cancel dismisses inline rename at both mobile widths and returns focus to row actions.');
    } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
