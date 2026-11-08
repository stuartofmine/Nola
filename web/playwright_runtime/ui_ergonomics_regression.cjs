// SPDX-License-Identifier: Apache-2.0
// Runs against the owned production Host after real_cross_platform_long_task.
// No generation or model response is mocked; checks input, focus, copy and layout.
const {chromium}=require('playwright-core');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const {query}=require('./real_model_evidence.cjs');
const base=process.env.NOLA_LONG_WEB_URL, out=process.env.NOLA_LONG_WEB_OUT;
(async()=>{
    const rows=query(out+'/store/nola.db',"SELECT conversation_id FROM runs WHERE kind='conversation' GROUP BY conversation_id ORDER BY MIN(created_at_ms)",[]);
    assert.ok(rows.length);
    const id=rows[0].conversation_id;
    const browser=await chromium.launch({executablePath:process.env.CHROME_PATH||'/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',headless:true});
    const results=[];
    let currentPage;
    try {
        for (const [width,height,scheme] of [[1440,960,'dark'],[390,844,'light'],[320,640,'dark'],[900,560,'light']]) {
            const context=await browser.newContext({viewport:{width,height},colorScheme:scheme,locale:'zh-CN',permissions:['clipboard-read','clipboard-write']});
            const page=await context.newPage(); currentPage=page; page.setDefaultTimeout(15000);
            const errors=[];page.on('pageerror',e=>errors.push(e.message));
            await page.goto(base+'/chat?conversation='+encodeURIComponent(id));
            if(new URL(page.url()).pathname==='/login') {
                await page.locator('input[type=password]').fill(fs.readFileSync(out+'/store/home/.nola/web/mobile-access-token','utf8').trim());
                await page.locator('button[type=submit]').click();
                await page.goto(base+'/chat?conversation='+encodeURIComponent(id));
            }
            const composer=page.locator('#chat-composer:not([disabled])');await composer.waitFor();
            // Dismiss narrow navigation without changing the selected owner.
            const panel=page.locator('dialog:modal');
            if(await panel.count())await page.keyboard.press('Escape');
            await composer.fill('Unsent ergonomics draft 草稿');await composer.focus();
            await page.keyboard.press('Control+f');
            await page.locator('#chat-search-input').waitFor();
            assert.equal(await page.locator('#chat-search-input').evaluate(e=>e===document.activeElement),true);
            await page.locator('#chat-search-input').fill('ergonomics');
            await page.keyboard.press('Escape');
            await page.waitForFunction(()=>document.activeElement?.id==='chat-composer');
            assert.equal(await composer.inputValue(),'Unsent ergonomics draft 草稿');
            const actions=page.locator('article:not(.user) .message-actions').first();
            const copy=actions.locator(':scope > button.copy-action');await copy.click();
            const status=actions.locator(':scope > span[role=status]');await status.waitFor();
            const copied=await page.evaluate(()=>navigator.clipboard.readText());assert.ok(copied.trim());
            await page.waitForTimeout(1000);await copy.click();await page.waitForTimeout(1000);
            assert.equal(await status.count(),1,'a previous copy timer must not clear the latest feedback');
            await status.waitFor({state:'detached',timeout:3000});
            const style=await copy.evaluate(e=>({background:getComputedStyle(e).backgroundColor,border:getComputedStyle(e).borderWidth}));
            assert.ok(style.background==='transparent'||style.background.endsWith(', 0)'));
            assert.equal(style.border,'0px');
            // Use a short actual viewport so a concise real answer still has
            // reading history. Do not fabricate extra messages to force overflow.
            await page.setViewportSize({width,height:360});
            const transcript=page.locator('.transcript');await transcript.focus();
            assert.equal(await transcript.evaluate(e=>e===document.activeElement),true);
            await page.keyboard.press('Home');
            await page.waitForTimeout(250);
            await page.waitForFunction(()=>document.querySelector('.transcript').scrollTop<2);
            const scrollable=await transcript.evaluate(e=>e.scrollHeight>e.clientHeight+24);
            assert.ok(scrollable,'the real conversation must have a scrollable reading history');
            await page.setViewportSize({width,height:480});
            await page.waitForTimeout(250);
            assert.ok(await transcript.evaluate(e=>e.scrollTop<2),'resize must preserve reading intent');
            await page.locator('.latest').click();
            await page.waitForFunction(()=>{const e=document.querySelector('.transcript');return e.scrollHeight-e.scrollTop-e.clientHeight<3;});
            await page.setViewportSize({width,height});
            await page.waitForFunction(()=>{const e=document.querySelector('.transcript');return e.scrollHeight-e.scrollTop-e.clientHeight<3;});
            await page.emulateMedia({reducedMotion:'reduce'});
            await page.waitForTimeout(150);
            assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),true);
            await page.screenshot({path:`${out}/ergonomics-${width}-${scheme}.png`});
            assert.deepEqual(errors,[]);
            results.push({width,height,scheme,focusRestored:true,copyFeedbackRenewed:true,readingPreserved:true,latestPreservedOnResize:true,overflow:false});
            await context.close();
        }
        fs.writeFileSync(out+'/ergonomics.json',JSON.stringify({passed:true,cases:results},null,2));
        console.log('PASS: search focus, draft, renewed clipboard feedback, keyboard scrolling and viewport reading ownership in four layouts');
    } catch (error) {
        if (currentPage && !currentPage.isClosed()) await currentPage.screenshot({path:out+'/ergonomics-failure.png'}).catch(()=>{});
        throw error;
    } finally {await browser.close();}
})().catch(error=>{console.error(error);process.exitCode=1;});
