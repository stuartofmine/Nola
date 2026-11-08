const {chromium} = require('playwright-core');
const assert = require('node:assert/strict');
const fs = require('node:fs');
(async () => {
    const browser = await chromium.launch({executablePath: process.env.CHROME_PATH || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome', headless:true});
    try {
        for (const width of [320, 390, 1440]) {
            const page = await browser.newPage({viewport:{width,height:900}, colorScheme:width===390?'dark':'light'});
            const errors=[], writes=[];
            let selected=false, failRead=false;
            page.on('pageerror', e=>errors.push(e.message));
            const inventory=()=>({accounts:[], selections:selected?[{connection_id:'fixture',catalog_key:'chat_completions:test',enabled:true,selected:false}]:[], connections:[{revision:1, document:{format_version:1, connection:{id:'fixture',display_name:'隔离连接 · 长名称的模型服务用于验证文字换行',base_url:'https://example.invalid/very-long-endpoint-address/v1',protocols:['chat_completions'],source:'manual',source_id:null,account_id:null,enabled:true},catalog:{observed_at_ms:1,models:[{model_id:'test',display_name:'长模型名称 Model for writing and reasoning',protocol:'chat_completions',context_window:131072,modalities:['text'],reasoning_levels:[]}]}}}]});
            await page.route('**/api/model-gateway/access', r=>r.fulfill({json:{revision:1,enabled:false}}));
            await page.route('**/api/model-connections', async route=>{
                if(route.request().method()==='POST') {
                    writes.push(route.request().postDataJSON());
                    if(writes.length===1) return route.fulfill({status:503,json:{error:'isolated_save_failure'}});
                    selected=true;
                    return route.fulfill({json:{}});
                }
                return route.fulfill(failRead?{status:503,json:{error:'isolated_read_failure'}}:{json:inventory()});
            });
            await page.goto(`${process.env.UI_FIXTURE_URL||'http://127.0.0.1:5129'}/models?tab=api`);
            const root=page.locator('.connections'), summary=root.locator('.connection-summary');
            await summary.waitFor();
            await summary.click({position:{x:8,y:15}});
            const checkbox=root.locator('.connection-body > .model input');
            await checkbox.waitFor();
            assert.equal(await checkbox.isChecked(),false);
            await checkbox.click();
            await root.locator(':scope > [role=alert]').waitFor();
            assert.equal(await checkbox.isChecked(),false,'failed save must not leave a browser-only enabled state');
            assert.equal(writes.length,1);
            assert.deepEqual(writes[0].enabled_model_ids,['chat_completions:test']);
            await checkbox.focus(); await page.keyboard.press('Space');
            await page.waitForFunction(()=>document.querySelector('.connection-body > .model input')?.checked);
            assert.equal(writes.length,2);
            failRead=true;
            await root.locator('.connections-heading button').click();
            await root.locator(':scope > [role=alert]').waitFor();
            assert.equal(await checkbox.isChecked(),true,'refresh failure retains the last confirmed inventory');
            failRead=false;
            await root.locator('.connections-heading button').click();
            await root.locator(':scope > [role=alert]').waitFor({state:'hidden'});
            const search=root.locator(':scope > label input');
            await search.fill('no-matching-fixture');
            await root.locator(':scope > [role=status]').waitFor();
            assert.equal(await root.locator('.connection-detail').count(),0);
            await search.fill('reasoning');
            await root.locator('.connection-detail').waitFor();
            await root.locator('.connection-summary').click();
            assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),true,'connection layout must not overflow');
            await page.waitForTimeout(250); // Capture the settled disclosure transition.
            if(process.env.UI_EVIDENCE_DIR){fs.mkdirSync(process.env.UI_EVIDENCE_DIR,{recursive:true});await root.screenshot({path:`${process.env.UI_EVIDENCE_DIR}/connections-${width}.png`});}
            assert.deepEqual(errors,[]);
            await page.close();
        }
        console.log('PASS: actual API models at 320/390/1440; long names, summary hit area, failed toggle rollback, keyboard save, stale inventory, search empty/result states');
    } finally {await browser.close();}
})().catch(e=>{console.error(e);process.exitCode=1;});
