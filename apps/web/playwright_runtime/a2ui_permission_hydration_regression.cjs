const {chromium}=require('playwright-core');
const fs=require('node:fs');
const assert=require('node:assert/strict');
(async()=>{
    const state=JSON.parse(fs.readFileSync((process.env.NOLA_A2UI_HOST_STATE || require('./test_environment.cjs').artifactPath('nola-a2ui-real-host-state.json')),'utf8'));
    const conversation=process.env.NOLA_A2UI_TEST_CONVERSATION;
    assert.ok(conversation);
    const browser=await chromium.launch({...require('./test_environment.cjs').browserOptions(),headless:true});
    try {
        for (const fault of ['slow','failed']) {
            const context=await browser.newContext();const page=await context.newPage();let intercepted=0;
            const original=JSON.parse(fs.readFileSync(process.env.NOLA_A2UI_CLIENT_IDENTITY_FILE || require('./test_environment.cjs').artifactPath('nola-a2ui-real-flow/client.json'),'utf8'));
            await page.addInitScript(id=>sessionStorage.setItem('nola.web.chat.client',id),original.client_id);
            await page.route('**/api/chat/agent/v2/journal',async route=>{
                if(route.request().postDataJSON()?.operation!=='open_snapshot')return route.continue();
                intercepted++;
                if(fault==='slow')await new Promise(resolve=>setTimeout(resolve,10000));
                await route.fulfill({status:503,json:{error:'isolated history hydration fault'}}).catch(()=>{});
            });
            await page.goto(state.url,{waitUntil:'domcontentloaded',timeout:60000});
            await page.goto(new URL('/chat?conversation='+encodeURIComponent(conversation),state.url).href,{waitUntil:'domcontentloaded',timeout:60000});
            const recovery=process.env.NOLA_A2UI_EXPECT_RECOVERY==='1';
            const panel=page.locator(recovery?'.recovery':'.permission-panel');
            await panel.waitFor({timeout:30000});
            if(!recovery)assert.equal((await panel.locator('.tool').innerText()).trim(),'render_ui');
            const action=panel.locator(recovery?'button':'.actions button.primary').first();
            assert.equal(await action.isEnabled(),true);
            await page.waitForTimeout(fault==='failed'?500:100);
            assert.ok(intercepted>0,'history fault must actually be exercised');
            assert.equal(await action.isEnabled(),true);
            console.log('PASS',recovery?'recovery':'permission','actionable while history',fault);
            await context.close();
        }
    } finally {await browser.close();}
})().catch(error=>{console.error('Hydration regression failure',error.name,error.message.replace(/https?:\/\/[^\s]+/g,'[URL]'));process.exitCode=1});
