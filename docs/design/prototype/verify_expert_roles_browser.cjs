// SPDX-License-Identifier: Apache-2.0
// All input goes through Browser Use; all data is synthetic.
'use strict';
const {chromium}=require('../../../apps/web/playwright_runtime/node_modules/playwright-core');
const {BrowserUseExecutor}=require('../../../apps/web/playwright_runtime/browser_use.cjs');
const environment=require('../../../apps/web/playwright_runtime/test_environment.cjs');
const assert=require('node:assert/strict'),fs=require('node:fs'),os=require('node:os'),path=require('node:path'),net=require('node:net');
const base=process.env.PROTOTYPE_URL||'http://127.0.0.1:8071';
assert.ok(['127.0.0.1','localhost','[::1]'].includes(new URL(base).hostname),'Only a local synthetic prototype is allowed');
const output=environment.artifactPath('expert-roles/prototype');
(async()=>{
    fs.mkdirSync(output,{recursive:true});
    const socket=net.createServer();await new Promise(r=>socket.listen(0,'127.0.0.1',r));const port=socket.address().port;await new Promise(r=>socket.close(r));
    const profile=fs.mkdtempSync(path.join(os.tmpdir(),'nola-expert-role-prototype-'));let context,executor;
    const receipt={scope:'synthetic-prototype-only',input:'browser-use',cases:[],errors:[]};
    try{
        context=await chromium.launchPersistentContext(profile,{...environment.browserOptions(),headless:true,viewport:{width:1440,height:900},reducedMotion:'reduce',args:[`--remote-debugging-port=${port}`]});
        const page=context.pages()[0];page.setDefaultTimeout(10000);
        await context.route('**/*',route=>new URL(route.request().url()).origin===new URL(base).origin?route.continue():route.abort());
        page.on('pageerror',error=>receipt.errors.push(error.stack||error.message));
        executor=await BrowserUseExecutor.attach(`http://127.0.0.1:${port}`,context);
        async function act(locator,operation='click',args={}){await page.waitForFunction(()=>!document.querySelector('.im-departing')&&document.getAnimations().every(a=>a.playState!=='running'||a.effect?.getTiming().iterations===Infinity));const element=await locator.elementHandle();assert.ok(element);try{await executor.perform(page,operation,args,element);}catch(error){throw new Error(`${operation} ${locator}: ${error.message}`,{cause:error});}finally{await element.dispose();}}
        async function select(locator,value){const values=await locator.locator('option').evaluateAll(xs=>xs.map(x=>x.value));assert.ok(values.includes(value));await act(locator,'type_text',{text:value});assert.equal(await locator.inputValue(),value);}
        async function openHistory(width) {
            if(width<700) await act(page.locator('[data-action="im:nav:im-details"]'));
            else {
                await act(page.locator('[data-action="sn-group:上下文"]'));
                const participants=page.locator('[data-action="dk-section:参与者"]');
                if(await participants.getAttribute('aria-expanded')!=='true') await act(participants);
            }
            const toggle=page.locator('[data-action="eh:toggle:design"]');
            if(await toggle.getAttribute('aria-expanded')!=='true') await act(toggle);
        }
        for(const width of [1440,390])for(const theme of ['light','dark']){
            await page.setViewportSize({width,height:900});await page.emulateMedia({colorScheme:theme});
            await page.addInitScript(value=>localStorage.setItem('nola-prototype-theme',value),theme);
            await executor.perform(page,'navigate',{url:base});
            if(width<700)await act(page.locator('[data-action="im:chat:design"]'));
            await act(page.locator('#compose'),'type_text',{text:'做代码审查 @代码'});
            await act(page.locator('[data-er-mention="expert-code"]'));
            assert.equal(await page.locator('#compose').inputValue(),'做代码审查 ');
            assert.equal(await page.locator('.er-expert-chip[data-expert-id="expert-code"]').count(),1);
            if(width<700){await act(page.locator('[data-action="im:nav:im-details"]'));await act(page.locator('[data-action="im:participants"]'));}
            else await act(page.locator('.conversation-controls [data-action="experts"]'));
            await act(page.locator('#dialog [data-action="er:manage"]'));
            assert.equal(await page.locator('#dialog [data-action^="er:builtin:"]').count(),7);
            const builtinCount=await page.evaluate(()=>window.expertRolesModule.snapshot().experts.length);
            await act(page.locator('#dialog [data-action^="er:builtin:"]').first());
            assert.equal(await page.locator('#dialog [data-action^="er:copy:"]').count(),1);
            await act(page.locator('#dialog [data-action="er:manage"]'));
            assert.equal(await page.evaluate(()=>window.expertRolesModule.snapshot().experts.length),builtinCount);
            await act(page.locator('[data-action="er:edit:expert-research"]'));
            await select(page.locator('#er-model'),'five-level');
            assert.equal(await page.locator('#er-effort option').count(),6,'Automatic plus the actual five declared levels');
            const role=await page.evaluate(()=>window.expertRolesModule.fixtures.catalog.templates[0].id);
            assert.equal(await page.locator('#er-role option').count(),8);
            await select(page.locator('#er-role'),role);
            for(const label of ['提示词','工具组合','配套 Skill'])await act(page.locator('#er-role-detail summary').filter({hasText:label}));
            await page.screenshot({path:path.join(output,`${width}-${theme}-role-preview.png`)});
            await act(page.locator('[data-action="er:apply-role"]'));await act(page.locator('[data-action="er:role-cancel"]'));
            assert.match(await page.locator('#er-current-template').innerText(),/自定义模板/);
            await act(page.locator('[data-action="er:apply-role"]'));await act(page.locator('[data-action="er:role-confirm"]'));
            await act(page.locator('[data-action="er:save"]'));
            const updated=await page.evaluate(()=>window.expertRolesModule.snapshot().experts.find(e=>e.id==='expert-research'));
            assert.equal(updated.defaults.selectedPromptTemplateID,role);assert.equal(updated.defaults.futureSetting,'preserved');assert.deepEqual(updated.defaults.modelOverrides,{'max-level':{temperature:.2}});
            await act(page.locator('#dialog [data-action="close"]').first());if(width<700)await act(page.locator('[data-action="im:back:chat"]'));
            const historyBefore=await page.evaluate(()=>current().messages.map(m=>({role:m.role,text:m.text})));
            await act(page.locator('#compose'),'type_text',{text:'继续 @研究'});await act(page.locator('[data-er-mention="expert-research"]'));
            await act(page.locator('[data-action="er:rejoin-cancel"]'));assert.equal(await page.locator('#compose').inputValue(),'继续 @研究');
            await act(page.locator('[data-er-mention="expert-research"]'));await act(page.locator('[data-action="er:rejoin-confirm"]'));
            assert.equal(await page.locator('#compose').inputValue(),'继续 ');
            assert.equal(await page.evaluate(()=>window.expertRolesModule.snapshot().retainedContexts.length),1);
            assert.deepEqual(await page.evaluate(()=>current().messages.map(m=>({role:m.role,text:m.text}))),historyBefore);
            if(!await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)){console.error('overflow',width,theme,await page.evaluate(()=>Array.from(document.querySelectorAll('body *')).map(e=>({tag:e.tagName,cls:e.className,x:e.getBoundingClientRect().x,right:e.getBoundingClientRect().right})).filter(e=>e.right>innerWidth+1).slice(0,25)));await page.screenshot({path:path.join(output,`${width}-${theme}-overflow-failure.png`)});}
            assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),true);
            await page.screenshot({path:path.join(output,`${width}-${theme}-mention-chip.png`)});
            assert.equal(await page.locator('.rail-history .eh-tree, .im-chat-list .eh-tree').count(),0);
            await openHistory(width);
            const previous=page.locator('.eh-children button').filter({hasText:'历史上下文 · 只读'}).first();
            await act(previous);
            assert.equal(await page.locator('#dialog textarea').count(),0,'History has no ordinary send composer');
            assert.equal(await page.locator('[data-action="eh:continue"]').count(),0,'Old context cannot silently rejoin');
            await page.screenshot({path:path.join(output,`${width}-${theme}-expert-old-history.png`)});
            await act(page.locator('#dialog [data-action="close"]').first());
            await act(page.locator('.eh-children button').filter({hasText:'研究'}).filter({hasText:'当前'}).first());
            await act(page.locator('[data-action="eh:continue"]'));
            assert.equal(await page.evaluate(()=>current().id),'design');
            assert.equal(await page.locator('.er-expert-chip[data-expert-id="expert-research"]').count(),1);
            await openHistory(width);
            const exchanges=await page.evaluate(()=>window.expertHistoryModule.snapshot().exchanges.filter(e=>e.parentID==='design'));
            assert.ok(exchanges.length,'The initial synthetic conversation includes an explicit shared Task cooperation receipt');
            const exchange=exchanges[0];assert.equal(exchange.taskID,exchange.exchangeID);
            await act(page.locator(`[data-action="eh:open:${encodeURIComponent('design')}:${encodeURIComponent(exchange.target.contextID)}"]`));
            assert.equal(await page.locator(`[data-peer-input="${exchange.exchangeID}"] small`).textContent(),exchange.source.name,'Peer input must retain its frozen expert author');
            await act(page.locator('.eh-exchanges summary').first());
            assert.ok((await page.locator('.eh-exchange-body').textContent()).includes(exchange.result));
            await page.screenshot({path:path.join(output,`${width}-${theme}-shared-task-cooperation.png`)});
            await act(page.locator('#dialog [data-action="close"]').first());
            if(width<700)await act(page.locator('[data-action="im:back:chat"]'));
            const beforeSend=await page.evaluate(()=>current().messages.length);const selectionBefore=await page.locator('.er-expert-chip').evaluateAll(xs=>xs.map(x=>x.dataset.expertId));const membersBefore=await page.evaluate(()=>window.expertRolesModule.snapshot().members);
            await act(page.locator('#compose'),'type_text',{text:'请审查请求重试和取消流程，重点检查旧回调覆盖与保存失败后的草稿恢复。'});assert.equal(await page.locator('#compose').inputValue(),'请审查请求重试和取消流程，重点检查旧回调覆盖与保存失败后的草稿恢复。');
            await act(page.locator(width<700?'[data-action="im:send"]':'#composer-form button[type="submit"]'));
            await page.waitForFunction(count=>current().messages.length===count+2,beforeSend,{timeout:20000});
            const newMessages=await page.evaluate(count=>current().messages.slice(count),beforeSend);
            assert.equal(newMessages[0].role,'user');assert.equal(newMessages[0].text,'请审查请求重试和取消流程，重点检查旧回调覆盖与保存失败后的草稿恢复。');
            assert.ok(newMessages[0].sendContext.mainExpertSelection.selected_experts.length);assert.equal(newMessages[1].role,'Nola');
            assert.ok(newMessages.every(message=>['user','Nola'].includes(message.role)),'Only Main speaks in the conversation');
            if(width<700&&await page.locator('.im-jump').isVisible()){await act(page.locator('.im-jump'));await page.waitForFunction(()=>{const e=document.querySelector('.im-transcript');return e.scrollHeight-e.scrollTop-e.clientHeight<2;});}
            assert.equal(await page.locator('#compose').inputValue(),'','Accepted original draft is consumed');
            assert.deepEqual(await page.locator('.er-expert-chip').evaluateAll(xs=>xs.map(x=>x.dataset.expertId)),selectionBefore,'Conversation expert selection remains until explicitly cancelled');
            assert.deepEqual(await page.evaluate(()=>window.expertRolesModule.snapshot().members),membersBefore,'Main send does not add duplicate members');
            const composerBox=await page.locator(width<700?'#im-composer':'#composer-form').boundingBox();
            if(!composerBox||composerBox.y<0||composerBox.y+composerBox.height>900)console.error('composer overflow',width,theme,composerBox,await page.evaluate(()=>({scrollY,height:innerHeight,body:document.body.scrollHeight,chat:document.querySelector('.im-chat')?.getBoundingClientRect().toJSON(),transcript:document.querySelector('#transcript')?.getBoundingClientRect().toJSON()})));
            assert.ok(composerBox&&composerBox.y>=0&&composerBox.y+composerBox.height<=900,'Composer remains fully inside the viewport');
            await page.waitForFunction(()=>!document.querySelector('#toast.visible'));
            await page.screenshot({path:path.join(output,`${width}-${theme}-main-unified-reply.png`)});
            receipt.cases.push({width,theme,flow:'role-preview-confirmation-declared-efforts-id-mention-rejoin-child-history-readonly-return-parent'});
        }
        assert.deepEqual(receipt.errors,[]);fs.writeFileSync(path.join(output,'receipt.json'),JSON.stringify(receipt,null,2));console.log(`PASS ${receipt.cases.length} expert prototype Browser Use cases`);
    }catch(error){console.error('page errors',receipt.errors);if(context){const p=context.pages()[0];console.error('page state',await p.evaluate(()=>({url:location.href,text:document.body.textContent.slice(0,350)})).catch(()=>null));}throw error;}finally{if(executor)await executor.close();if(context)await context.close();fs.rmSync(profile,{recursive:true,force:true});}
})().catch(error=>{console.error(error);process.exitCode=1;});
