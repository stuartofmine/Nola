const {chromium}=require('playwright-core');
const assert=require('node:assert/strict');
const fs=require('node:fs');
(async()=>{
    const realChat = process.env.UI_REAL_CHAT_URL;
    if (realChat) assert.ok(['127.0.0.1', 'localhost'].includes(new URL(realChat).hostname), 'Use an owned loopback test Host');
    const browser=await chromium.launch({...require('./test_environment.cjs').browserOptions(),headless:true});
    try {
        for(const width of [320,1440]) {
            const page=await browser.newPage({viewport:{width,height:900},hasTouch:width===320});
            const errors=[];let mutations=0;
            page.on('pageerror',e=>errors.push(e.message));
            await page.addInitScript(()=>Object.defineProperty(navigator,'clipboard',{configurable:true,value:{writeText:async()=>{throw new Error('Clipboard unavailable in fixture');}}}));
            page.on('request',r=>{if(r.method()==='POST'&&/\/(branch|truncate)$/.test(r.url()))mutations++;});
            await page.goto(realChat || `${process.env.UI_FIXTURE_URL||'http://127.0.0.1:5129'}/chat?conversation=review`);
            const actions=page.locator('article:not(.user) .message-actions').first(), summary=actions.locator('summary');
            await summary.waitFor();
            await page.locator('.conversation[aria-current]').waitFor({state:'attached'});
            assert.equal(await actions.locator('button:visible').count(),1,'only copy is permanent');
            for (const row of await page.locator('.message-actions').all()) {
                const trigger = row.locator('summary');
                assert.ok((await trigger.getAttribute('aria-label')).trim());
                const [box, previous] = await row.evaluate(el => [
                    el.querySelector('summary'), [...el.querySelectorAll(':scope > button')].at(-1)
                ].map(button => button.getBoundingClientRect().toJSON()));
                assert.ok(Math.abs(box.y + box.height/2 - previous.y - previous.height/2) <= 2, 'overflow is next to primary actions');
                assert.ok(box.width >= (width === 320 ? 44 : 32) && box.width <= 48);
                assert.ok(box.height >= (width === 320 ? 44 : 32));
                assert.deepEqual(await trigger.evaluate(e=>[getComputedStyle(e).borderTopWidth,getComputedStyle(e).backgroundColor,getComputedStyle(e,'::after').content]),
                    ['0px','rgba(0, 0, 0, 0)','none']);
            }
            if(process.env.UI_EVIDENCE_DIR){
                fs.mkdirSync(process.env.UI_EVIDENCE_DIR,{recursive:true});
                await page.screenshot({path:`${process.env.UI_EVIDENCE_DIR}/message-actions-${width}-closed.png`});
            }
            await summary.focus();await page.keyboard.press('Enter');
            const menu=actions.locator('.disclosed-actions:popover-open');
            await menu.waitFor();
            const box=await menu.boundingBox();
            assert.ok(box.x >= 0 && box.x+box.width <= width && box.y >= 0 && box.y+box.height <= 900);
            await page.keyboard.press('Tab');
            assert.ok(await menu.locator('button').first().evaluate(e=>e===document.activeElement));
            if(process.env.UI_EVIDENCE_DIR){
                fs.mkdirSync(process.env.UI_EVIDENCE_DIR,{recursive:true});
                await page.waitForTimeout(250);
                await page.screenshot({path:`${process.env.UI_EVIDENCE_DIR}/message-actions-${width}.png`});
            }
            await page.keyboard.press('Enter');
            assert.equal(await actions.locator('details').evaluate(e=>e.open),false, 'choosing select dismisses the popup');
            assert.equal(await summary.evaluate(e=>e===document.activeElement),true);
            await summary.press('Space');
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
            assert.ok(realChat ? (await manual.inputValue()).trim() : (await manual.inputValue()).includes('验收用回答'));
            if (realChat) {
                const process = page.locator('.execution-process').first(), processHeader=process.locator(':scope > summary');
                await processHeader.focus(); await page.keyboard.press('Space');
                await page.waitForFunction(()=>document.querySelector('.execution-process')?.open);
                assert.equal(await processHeader.locator('.chevron').count(),1);
                assert.equal(await processHeader.evaluate(e=>getComputedStyle(e,'::after').content),'none');
                const tool=process.locator('.tool-part > summary').first();
                await tool.waitFor();
                assert.equal(await tool.evaluate(e=>getComputedStyle(e,'::after').content),'none');
                await processHeader.press('Enter');
                assert.equal(await process.evaluate(e=>e.open),false);
                assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),true);
                assert.deepEqual(errors,[]);
                await page.close();
                continue;
            }
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
        console.log('PASS: compact and desktop message action layout, popup placement, keyboard, focus, cancel and clipboard fallback');
    } finally {await browser.close();}
})().catch(e=>{console.error(e);process.exitCode=1;});
