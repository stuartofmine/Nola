const {chromium}=require('playwright-core');
const assert=require('node:assert/strict');
const fs=require('node:fs');
(async()=>{
    const browser=await chromium.launch({...require('./test_environment.cjs').browserOptions(),headless:true});
    try {
        for(const width of [320,1440]) {
            const page=await browser.newPage({viewport:{width,height:900}});
            const errors=[];let mutations=0;
            page.on('pageerror',e=>errors.push(e.message));
            await page.addInitScript(()=>Object.defineProperty(navigator,'clipboard',{configurable:true,value:{writeText:async()=>{throw new Error('Clipboard unavailable in fixture');}}}));
            page.on('request',r=>{if(r.method()==='POST'&&/\/(branch|truncate)$/.test(r.url()))mutations++;});
            await page.goto(`${process.env.UI_FIXTURE_URL||'http://127.0.0.1:5129'}/chat?conversation=review`);
            const actions=page.locator('.message-actions').first(), summary=actions.locator('summary');
            await summary.waitFor();
            await page.locator('.conversation[aria-current]').waitFor({state:'attached'});
            assert.equal(await actions.locator('button:visible').count(),1,'only copy is permanent');
            await summary.focus();await page.keyboard.press('Enter');
            if(process.env.UI_EVIDENCE_DIR){
                fs.mkdirSync(process.env.UI_EVIDENCE_DIR,{recursive:true});
                await page.waitForTimeout(250);
                await page.screenshot({path:`${process.env.UI_EVIDENCE_DIR}/message-actions-${width}.png`});
            }
            // The last disclosed action truncates after the selected persisted message.
            await actions.locator('.disclosed-actions button').last().click();
            await page.locator('dialog:modal').waitFor();
            await page.keyboard.press('Escape');
            await page.waitForFunction(()=>document.activeElement?.matches('.message-actions summary'));
            assert.equal(await actions.locator('details').evaluate(e=>e.open),false);
            assert.equal(mutations,0,'cancel sends no mutation');
            await summary.click();await page.keyboard.press('Escape');
            assert.equal(await summary.evaluate(e=>e===document.activeElement),true);
            await actions.locator(':scope > button').click();
            const manual=actions.locator('textarea');await manual.waitFor();
            assert.equal(await manual.evaluate(e=>e===document.activeElement),true,'clipboard fallback is keyboard ready');
            assert.equal(await manual.evaluate(e=>e.selectionEnd-e.selectionStart===e.value.length),true);
            assert.ok((await manual.inputValue()).includes('验收用回答'));
            assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),true);
            await page.route('**/api/model-profiles', async route => {
                const response = await route.fetch();
                const data = await response.json();
                await route.fulfill({json:{profiles:data.profiles.map(profile=>({...profile,provider:'openai-compatible'}))}});
            });
            // The real API configuration route must focus a field after mounting.
            await page.goto(`${process.env.UI_FIXTURE_URL||'http://127.0.0.1:5129'}/models?tab=api&configure=fixture-model`);
            const editor=page.locator('dialog:modal');await editor.waitFor();
            const name=editor.locator('input').first();
            assert.equal(await name.evaluate(e=>e===document.activeElement),true);
            const initial=await name.inputValue();await name.fill('Unsaved fixture name');
            await page.keyboard.press('Escape');
            await page.locator('dialog:modal').last().locator('button').first().click();
            assert.equal(await name.inputValue(),'Unsaved fixture name');
            await name.fill(initial);await page.keyboard.press('Escape');
            assert.equal(await page.locator('dialog:modal').count(),0);
            assert.deepEqual(errors,[]);
            await page.close();
        }
        console.log('PASS: compact/desktop action disclosure, cancel without mutation, restored focus, blocked clipboard manual selection, remote model initial focus and dirty cancel');
    } finally {await browser.close();}
})().catch(e=>{console.error(e);process.exitCode=1;});
