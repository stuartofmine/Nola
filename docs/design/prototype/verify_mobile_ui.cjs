// SPDX-License-Identifier: Apache-2.0
// Mobile prototype geometry, visual hierarchy and real controls; no app/model access.
const {chromium}=require('../../../apps/web/playwright_runtime/node_modules/playwright-core');
const env=require('../../../apps/web/playwright_runtime/test_environment.cjs');
const fs=require('node:fs'),assert=require('node:assert/strict');
const base=process.env.PROTOTYPE_URL||'http://127.0.0.1:8071';
const out=process.env.PROTOTYPE_OUT||env.artifactPath('nola-mobile-ui-20261111');
(async()=>{
    fs.mkdirSync(out,{recursive:true});const browser=await chromium.launch({...env.browserOptions(),headless:true});
    const report={kind:'prototype-only',cases:[],screens:[],errors:[],externalRequests:[]};
    try {
        for(const [width,os,theme] of [[320,'ios','light'],[320,'ios','dark'],[390,'ios','light'],[390,'ios','dark'],[390,'android','light'],[390,'android','dark']]){
            const context=await browser.newContext({viewport:{width,height:900},colorScheme:theme,reducedMotion:'reduce',isMobile:true,hasTouch:true});
            const page=await context.newPage();page.setDefaultTimeout(8000);
            await context.route('**/*',r=>new URL(r.request().url()).origin===base?r.continue():(report.externalRequests.push(r.request().url()),r.abort()));
            page.on('pageerror',e=>report.errors.push({width,os,theme,message:e.stack}));
            const click=a=>page.locator(`[data-action="${a}"]:visible`).last().click();
            async function shot(name){await page.locator('#toast.visible').waitFor({state:'hidden'});await page.waitForTimeout(100);assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),name+' page overflow');const file=`${os}-${width}-${theme}-${name}.png`;await page.screenshot({path:out+'/'+file});report.screens.push(file);}
            await page.goto(base);
            if(os==='android'){await click('im:nav:settings');await page.locator('.im-preview-tools>summary').click();await click('im:platform');await click('im:nav:conversations');}
            assert.equal(await page.locator('[data-action="im:new"]:visible').count(),1);
            assert.ok(await page.locator('.im-heading h1').evaluate(e=>parseFloat(getComputedStyle(e).fontSize)>=24));
            await shot('conversations');
            await page.getByRole('button',{name:/周末，去山里走走/}).click();
            assert.equal(await page.locator('.im-title-avatar:visible').count(),0);
            assert.equal(await page.locator('.im-message-time:visible').count(),0);
            assert.equal(await page.locator('.im-message:not(.im-outgoing) .im-bubble').first().evaluate(e=>getComputedStyle(e).backgroundColor),'rgba(0, 0, 0, 0)');
            const min=os==='android'?48:44;
            for(const button of await page.locator('.im-chat-header .im-icon,.im-composer-toolbar button').all()){
                const rect=await button.boundingBox();assert.ok(rect&&rect.width>=min-.5&&rect.height>=min-.5,'Composer/header hit size');
            }
            const composer=await page.locator('.im-composer').boundingBox();assert.ok(composer.height<=132,'Initial composer must stay compact');
            await page.locator('#compose').fill('草稿在详情与设置之间保留');await shot('chat');
            await click('im:nav:im-details');await shot('conversation-details');await click('im:back:chat');
            assert.equal(await page.locator('#compose').inputValue(),'草稿在详情与设置之间保留');
            await click('im:back:conversations');await click('im:nav:settings');
            for(const group of await page.locator('.im-stage[data-route="settings"]>.im-screen>.im-group').all()){
                assert.equal(await group.evaluate(e=>getComputedStyle(e).borderTopWidth),'0px');
            }
            await shot('settings');await click('im:nav:im-general');await shot('general');
            await click('im:back:settings');await click('im:nav:models');
            assert.equal(await page.getByRole('button',{name:'添加连接',exact:true}).count(),0,'Configured models use one add action in the header');
            await shot('models');await click('im:nav:im-add-model');await shot('add-model');
            await click('im:back:models');await click('im:back:settings');await click('im:nav:information');await shot('library');
            const colors=await page.evaluate(()=>{const s=getComputedStyle(document.documentElement);return {text:s.getPropertyValue('--im-text').trim(),muted:s.getPropertyValue('--im-muted').trim(),background:s.getPropertyValue('--im-bg').trim()}});
            function luminance(hex){let rgb=hex.slice(1);if(rgb.length===3)rgb=rgb.split('').map(x=>x+x).join('');return [.2126,.7152,.0722].reduce((n,w,i)=>{let c=parseInt(rgb.slice(i*2,i*2+2),16)/255;return n+w*(c<=.04045?c/12.92:((c+.055)/1.055)**2.4)},0)}
            const b=luminance(colors.background);for(const color of [colors.text,colors.muted]){const f=luminance(color);assert.ok((Math.max(f,b)+.05)/(Math.min(f,b)+.05)>=4.5,'Body and metadata text contrast');}
            report.cases.push({width,os,theme,passed:true});await context.close();
        }
        const previewContext=await browser.newContext({viewport:{width:1100,height:1060},reducedMotion:'reduce'});
        await previewContext.route('**/*',r=>new URL(r.request().url()).origin===base?r.continue():(report.externalRequests.push(r.request().url()),r.abort()));
        const preview=await previewContext.newPage();preview.on('pageerror',e=>report.errors.push({preview:e.stack}));
        await preview.goto(base+'/mobile.html');const phone=preview.frameLocator('#phone');
        await phone.getByRole('button',{name:/周末，去山里走走/}).click();await phone.locator('#compose').fill('切换外观不丢草稿');
        await preview.locator('#theme').selectOption('dark');await preview.locator('#platform').selectOption('android');await preview.locator('#width').selectOption('320');
        assert.equal(await phone.locator('#compose').inputValue(),'切换外观不丢草稿');
        assert.equal(await phone.locator('.im-stage').getAttribute('data-os'),'android');
        assert.equal(await phone.locator('html').getAttribute('data-theme'),'dark');
        await preview.screenshot({path:out+'/desktop-mobile-preview.png'});report.screens.push('desktop-mobile-preview.png');
        await previewContext.close();
        assert.deepEqual(report.errors,[]);assert.deepEqual(report.externalRequests,[]);console.log(`PASS ${report.cases.length} mobile layouts, ${report.screens.length} captures`);
    }finally{fs.writeFileSync(out+'/receipt.json',JSON.stringify(report,null,2));await browser.close()}
})().catch(error=>{console.error(error);process.exitCode=1});
