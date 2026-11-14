// SPDX-License-Identifier: Apache-2.0
// Local prototype acceptance; external requests are blocked. No product or model calls.
const {chromium}=require('../../../apps/web/playwright_runtime/node_modules/playwright-core');
const env=require('../../../apps/web/playwright_runtime/test_environment.cjs');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const base=process.env.PROTOTYPE_URL||'http://127.0.0.1:8071';
const out=process.env.PROTOTYPE_OUT||env.artifactPath('nola-sidebar-prototype-20261111');
(async()=>{
    fs.mkdirSync(out,{recursive:true});
    const browser=await chromium.launch({...env.browserOptions(),headless:true});
    const receipt={kind:'prototype-only',cases:[],screens:[],errors:[],externalRequests:[]};
    try {
        for(const width of [1000,1440])for(const theme of ['light','dark']){
            const context=await browser.newContext({viewport:{width,height:960},colorScheme:theme,reducedMotion:'no-preference'});
            const page=await context.newPage();page.setDefaultTimeout(8000);
            await page.addInitScript(theme=>localStorage.setItem('nola-prototype-theme',theme),theme);
            await context.route('**/*',route=>new URL(route.request().url()).origin===base?route.continue():(receipt.externalRequests.push(route.request().url()),route.abort()));
            page.on('pageerror',error=>receipt.errors.push(error.message));
            const click=action=>page.locator(`[data-action="${action}"]:visible`).last().click();
            async function shot(name){
                await page.waitForTimeout(220);
                await page.locator('#toast.visible').waitFor({state:'hidden'});
                assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),name+' horizontal overflow');
                const file=`${width}-${theme}-${name}.png`;await page.screenshot({path:`${out}/${file}`});receipt.screens.push(file);
            }
            await page.goto(base+'/?stream=hold');
            assert.equal(await page.locator('[aria-label="左侧列表类别"] button').count(),2);
            assert.equal(await page.locator('[data-action="sn-left:activity"]').count(),0);
            assert.equal(await page.locator('.rail [data-action="nav:settings"]').count(),1);
            await click('sn-left:manage');
            assert.equal(await page.locator('.rail-resource-links>button').count(),5);
            for(const removed of ['activity','memory','settings'])assert.equal(await page.locator(`[data-action="sn-open:${removed}"]`).count(),0);
            await shot('resources');
            await click('sn-open:plans');
            assert.equal(await page.locator('.rail-resource-links [aria-current="page"]').getAttribute('data-action'),'sn-open:plans');
            await click('sn-open:data');
            for(const section of ['memory','plans','data']){
                await click('dk-data-pane:'+section);
                assert.equal(await page.locator(`[data-action="dk-data-pane:${section}"]`).getAttribute('aria-pressed'),'true');
                assert.equal(await page.locator('.rail-resource-links [aria-current="page"]').getAttribute('data-action'),'sn-open:data');
            }
            await shot('data-center');
            await click('sn-open:access');
            assert.equal(await page.evaluate(()=>manageModule.snapshot().capTab),'系统访问');
            await click('sn-open:capabilities');
            assert.equal(await page.evaluate(()=>manageModule.snapshot().capTab),'技能');
            await click('dk-content:chat');
            for(const [groupLabel,names] of [['任务',['工作概览','进度','目标']],['产物',['审阅','产物']],['上下文',['会话配置','上下文预算','执行','参与者']]]){
                if(await page.locator(`[data-action="sn-group:${groupLabel}"]`).getAttribute('aria-pressed')!=='true')await click('sn-group:'+groupLabel);
                assert.deepEqual(await page.locator('[data-inspector-section]').evaluateAll(nodes=>nodes.map(node=>node.dataset.inspectorSection)),names);
                for(const name of names){
                    for(const open of await page.locator('.inspector-section-toggle[aria-expanded=true]').evaluateAll(nodes=>nodes.map(node=>node.dataset.action)))await click(open);
                    await click('dk-section:'+name);
                    if(name==='工作概览'){
                        assert.equal(await page.locator('.work-links').count(),0);
                        const owner=await page.evaluate(()=>activeId);
                        await click('dk-workspace');await click('dk-workspace-set:演示项目');
                        assert.equal(await page.evaluate(()=>activeId),owner);
                        assert.ok((await page.locator('.prototype-workspace').innerText()).includes('演示项目'));
                        assert.equal(await page.locator('[data-action="conversation-workspace"]').count(),0);
                    }
                    if(name==='会话配置'){
                        assert.ok((await page.locator('.prototype-workspace').innerText()).includes('演示项目'));
                    }
                    if(name==='上下文预算'){
                        assert.equal(await page.locator('.inspector-knowledge details[open]').count(),0);
                        await page.locator('.inspector-knowledge summary').filter({hasText:/^Wiki$/}).click();
                        await page.getByLabel('Wiki草稿',{exact:true}).fill('SIDEBAR-DRAFT');
                        await click('dk-section:上下文预算');await click('dk-section:上下文预算');
                        assert.equal(await page.getByLabel('Wiki草稿',{exact:true}).inputValue(),'SIDEBAR-DRAFT');
                    }
                    if(name==='执行'){
                        const history=page.locator('[data-process-history]');
                        assert.equal(await history.getAttribute('open'),null);
                        assert.equal(await history.locator(':scope>summary').innerText(),'执行过程');
                        await history.locator(':scope>summary').click();
                        await click('dk-browser');
                        assert.notEqual(await page.locator('[data-process-history]').getAttribute('open'),null);
                        assert.equal(await page.locator('.studio-inspector [data-action="nav:activity"]').count(),0);
                        assert.equal(await page.locator('.studio-inspector [data-action="nav:capabilities"]').count(),0);
                    }
                    await page.locator(`[data-inspector-section="${name}"]`).scrollIntoViewIfNeeded();await shot('inspector-'+name);
                }
            }
            await click('sn-left:conversations');
            const owner=await page.evaluate(()=>conversations.find(c=>!c.messages.some(m=>m.permission==='pending')).id);
            await click('dk-select:'+owner);
            await page.locator('#compose').fill('检查原型的流式状态');
            await page.locator('#composer-form button[type=submit]').click();
            const status=page.locator('.flowing-status').first();await status.waitFor();
            assert.equal(await status.locator('.flow-highlight').evaluate(e=>getComputedStyle(e).animationName),'status-flow');
            await shot('processing-flow');
            await page.emulateMedia({reducedMotion:'reduce'});
            assert.equal(await status.locator('.flow-highlight').evaluate(e=>getComputedStyle(e).display),'none');
            await click('stop');
            receipt.cases.push({width,theme,passed:true});await context.close();
        }
        assert.deepEqual(receipt.errors,[]);assert.deepEqual(receipt.externalRequests,[]);
        console.log(`PASS ${receipt.cases.length} sidebar layouts, ${receipt.screens.length} captures`);
    } finally {fs.writeFileSync(out+'/receipt.json',JSON.stringify(receipt,null,2));await browser.close();}
})().catch(error=>{console.error(error);process.exitCode=1;});
