// Actual isolated Host: a rejected action is observed, explicitly cancelled, never redispatched.
const {chromium}=require('playwright-core');
const fs=require('node:fs');
const assert=require('node:assert/strict');
(async()=>{
    const state=JSON.parse(fs.readFileSync((process.env.NOLA_A2UI_HOST_STATE || require('./test_environment.cjs').artifactPath('nola-a2ui-real-host-state.json')),'utf8'));
    const cid=process.env.NOLA_A2UI_TEST_CONVERSATION;
    const clientFile=process.env.NOLA_A2UI_CLIENT_FILE;
    assert.ok(cid&&clientFile);
    assert.equal(new URL(state.url).hostname,'127.0.0.1');
    const client=JSON.parse(fs.readFileSync(clientFile,'utf8')).client_id;
    const out=require('./test_environment.cjs').artifactPath('nola-a2ui-rejected-queue-browser');fs.mkdirSync(out,{recursive:true,mode:0o700});
    const save=(name,value)=>fs.writeFileSync(`${out}/${name}.json`,JSON.stringify(value,null,2),{mode:0o600});
    const browser=await chromium.launch({...require('./test_environment.cjs').browserOptions(),headless:true});
    const page=await browser.newPage({viewport:{width:1280,height:900}});
    await page.addInitScript(value=>sessionStorage.setItem('nola.web.chat.client',value),client);
    let dispatches=0;const errors=[];
    page.on('request',request=>{if(new URL(request.url()).pathname==='/api/chat/agent/v2/stream')dispatches++;});
    page.on('pageerror',error=>errors.push(error.name));
    try{
        await page.goto(state.url,{waitUntil:'domcontentloaded'});
        const host=await page.evaluate(async()=>(await(await fetch('/api/web/session')).json()).hostId);
        const path=`/api/chat/conversations/${encodeURIComponent(cid)}/queued-input?${new URLSearchParams({expected_host_id:host,client_id:client})}`;
        const read=()=>page.evaluate(async path=>{const r=await fetch(path);if(!r.ok)throw Error(`queue ${r.status}`);return r.json();},path);
        const before=await read();assert.equal(before.dispatch.phase,'rejected');assert.ok(before.head);
        save('before',{command_id:before.head.command_id,phase:before.dispatch.phase});
        await page.goto(`${state.url}/chat?conversation=${encodeURIComponent(cid)}`,{waitUntil:'domcontentloaded'});
        const panel=page.locator('.queued-action');await panel.waitFor({timeout:30000});
        assert.ok((await panel.innerText()).includes('未执行'));
        await page.screenshot({path:`${out}/rejected.png`});
        const response=page.waitForResponse(r=>new URL(r.url()).pathname.endsWith('/queued-input/cancel'));
        await panel.getByRole('button',{name:'取消这次排队操作',exact:true}).click();
        const cancelled=await response;assert.equal(cancelled.status(),200);
        assert.equal((await cancelled.json()).command_id,before.head.command_id);
        await panel.waitFor({state:'hidden',timeout:30000});
        assert.equal((await read()).head,null);
        const observed=await page.evaluate(async receipt=>{
            const r=await fetch('/api/chat/agent/v2/submission',{method:'POST',headers:{'Content-Type':'application/json'},
                body:JSON.stringify({operation:'observe',identity:receipt.identity,request_digest:receipt.request_digest})});
            return {status:r.status,body:await r.json()};
        },before.dispatch);
        assert.equal(observed.status,200);
        assert.ok(JSON.stringify(observed.body).includes('rejected'),'original rejection remains observable');
        assert.equal(dispatches,0,'cancellation never launches another model turn');assert.deepEqual(errors,[]);
        await page.reload({waitUntil:'domcontentloaded'});
        await page.locator('#chat-composer').waitFor({timeout:30000});
        assert.equal((await read()).head,null);assert.equal(dispatches,0);
        await page.locator('.surface-reference input').first().waitFor({timeout:30000});
        await page.waitForFunction(() => {
            const button = document.querySelector('.surface-reference button.a2ui-button');
            return button && !button.disabled;
        }, {}, {timeout:30000});
        assert.ok(await page.locator('.surface-reference input').first().isEnabled());
        await page.screenshot({path:`${out}/cancelled.png`});
        save('result',{passed:true,conversation:cid,command_id:before.head.command_id,dispatches,receipt_retained:true,errors});
        console.log('PASS rejected queued action cancellation without redispatch');
    }catch(error){save('result',{passed:false,error:error.name,dispatches,errors});await page.screenshot({path:`${out}/failure.png`}).catch(()=>{});throw error;}
    finally{await browser.close();}
})().catch(error=>{console.error(error.name,error.message);process.exitCode=1;});
