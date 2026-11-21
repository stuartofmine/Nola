const {chromium}=require('playwright-core');
const assert=require('node:assert/strict');
const path=require('node:path');
(async()=>{
    const browser=await chromium.launch({...require('./test_environment.cjs').browserOptions(),headless:true});
    try {
        for(const width of [320,1440]) {
            const page=await browser.newPage({viewport:{width,height:900},locale:'en'});const errors=[];let writes=0;
            page.on('pageerror',e=>errors.push(e.message));
            page.on('request',r=>{if(r.method()==='POST'&&/\/api\/model-(profiles|endpoints)/.test(r.url()))writes++;});
            const endpoint = {id:'manual-endpoint', name:'Fixture endpoint', baseURL:'https://example.invalid/v1',
                wireProtocol:'chat_completions', enabled:true, order:0};
            const profile = {id:'manual-model', provider:'openai-compatible', modelName:'fixture', displayName:'Fixture',
                revision:1, enabled:true, selected:true, priority:50, order:0, endpoint, capabilities:['chat'], reasoning:null,
                executionCapabilities:{toolCalls:true, imageInput:false, reasoning:false, samplingParameters:false}, parameters:{}};
            await page.route('**/api/model-profiles', route => route.fulfill({json:{profiles:[profile]}}));
            await page.goto(`${process.env.UI_FIXTURE_URL||'http://127.0.0.1:5129'}/models?tab=api`);
            const create=page.getByRole('button',{name:'Add endpoint and model',exact:true});
            for(const change of ['name','address','protocol','capability']) {
                await create.click();
                const editor=page.locator('dialog:modal').first();
                const first=editor.locator('input').first();
                assert.equal(await first.evaluate(e=>e===document.activeElement),true);
                let control,original;
                if(change==='name') {control=first;original=await control.inputValue();await control.fill('Unsaved endpoint');}
                if(change==='address') {control=editor.locator('input[type=url]');original=await control.inputValue();await control.fill('https://example.invalid/isolated');}
                if(change==='protocol') {control=editor.locator('select').first();original=await control.inputValue();await control.selectOption('responses');}
                if(change==='capability') {control=editor.locator('input[type=checkbox]').last();original=await control.isChecked();await control.click();}
                assert.equal(await page.evaluate(()=>window.dispatchEvent(new Event('beforeunload',{cancelable:true}))),false,'dirty form guards leaving the page');
                await page.keyboard.press('Escape');
                assert.equal(await page.locator('dialog:modal').count(),2,`${change} alone needs discard confirmation`);
                await page.locator('dialog:modal').last().locator('button').first().click();
                assert.equal(await page.locator('dialog:modal').count(),1);
                if(change==='capability'){assert.notEqual(await control.isChecked(),original);await control.click();}
                else if(change==='protocol'){assert.equal(await control.inputValue(),'responses');await control.selectOption(original);}
                else {assert.notEqual(await control.inputValue(),original);await control.fill(original);}
                assert.equal(await page.evaluate(()=>window.dispatchEvent(new Event('beforeunload',{cancelable:true}))),true,'restoring baseline clears dirty state');
                await page.keyboard.press('Escape');
                assert.equal(await page.locator('dialog:modal').count(),0);
                assert.equal(await create.evaluate(e=>e===document.activeElement),true);
            }
            await page.locator('.endpoint-summary').first().click();
            const header=page.locator('.endpoint-head').first();
            if(width===320) await header.locator('.compact-actions > summary').click();
            const edit=header.getByRole('button',{name:/编辑端点|Edit endpoint/,exact:true}).filter({visible:true});
            await edit.click();
            const editor=page.locator('dialog:modal');await editor.waitFor();
            assert.equal(await editor.locator('input').first().evaluate(e=>e===document.activeElement),true);
            if (process.env.UI_EVIDENCE_DIR) await page.screenshot({path:path.join(process.env.UI_EVIDENCE_DIR,'endpoint-editor-'+width+'.png')});
            await page.keyboard.press('Escape');
            const returnTarget = width === 320 ? header.locator('.compact-actions > summary') : edit;
            assert.equal(await returnTarget.evaluate(e=>e===document.activeElement),true);
            assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),true);
            assert.equal(writes,0,'cancel and draft editing never persist a catalog');
            assert.deepEqual(errors,[]);await page.close();
        }
        console.log('PASS: 320/1440 creation guards name/address/protocol/capability drafts, cancel preservation, restored baseline, unload guard, initial and restored editor focus; no writes');
    }finally{await browser.close();}
})().catch(e=>{console.error(e);process.exitCode=1;});
