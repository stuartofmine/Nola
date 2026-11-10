const{chromium}=require('playwright-core');const assert=require('node:assert/strict');
(async()=>{const b=await chromium.launch({...require('./test_environment.cjs').browserOptions(),headless:true});try{for(const width of (process.env.UI_WIDTHS||'320,390,768,1440').split(',').map(Number))for(const height of [240,300,700]){
const p=await b.newPage({viewport:{width,height}});p.on('pageerror',e=>console.log('PAGEERROR',width,height,e.message));p.on('requestfailed',r=>console.log('REQUESTFAILED',r.url(),r.failure()));p.on('response',r=>{if(r.status()>=400)console.log('HTTPFAIL',r.status(),r.url())});p.on('console',m=>{if(m.type()==='error')console.log('CONSOLE',m.text())});await p.goto(`${process.env.UI_FIXTURE_URL||'http://127.0.0.1:5139'}/chat?conversation=review`);await p.locator('#chat-composer').waitFor().catch(async e=>{await p.screenshot({path:require('./test_environment.cjs').artifactPath(`nola-web-layers/failure-${width}-${height}.png`)});throw e;});await p.waitForTimeout(250);
await p.locator('.model-menu summary').click();await p.waitForTimeout(250);let r=await p.locator('.choices').boundingBox();console.log(width,height,'model',r);await p.screenshot({path:require('./test_environment.cjs').artifactPath(`nola-web-layers/model-${width}-${height}.png`)});
assert.ok(r.y>=0&&r.y+r.height<=height,`model menu clipped ${width}x${height}`);
assert.equal(await p.locator('.choices').evaluate(e=>{let r=e.getBoundingClientRect();return e.contains(document.elementFromPoint(r.x+r.width/2,r.y+Math.min(r.height-8,100)))}),true,'model choices must own hit testing over transcript');
await p.locator('.model-menu summary').press('End');assert.equal(await p.locator('.choices a').evaluate(e=>{let r=e.getBoundingClientRect();return r.bottom<=innerHeight&&e.contains(document.elementFromPoint(r.x+r.width/2,r.y+r.height/2))}),true);
await p.keyboard.press('Escape');await p.locator('.chat-header [aria-controls="chat-details"]').last().click();await p.waitForTimeout(250);
let dialog=p.locator('dialog:modal');if(await dialog.count()){r=await dialog.boundingBox();assert.ok(r.y>=0&&r.y+r.height<=height);await dialog.locator('.panel-toolbar button').last().focus();await p.keyboard.press('Escape');assert.equal(await p.locator('.chat-header [aria-controls="chat-details"]').last().evaluate(e=>e===document.activeElement),true);}
// Open the sidebar's delete confirmation, then cancel: no decision/write submitted.
if(width<=760){await p.locator('.navigation-toggle').click();await p.waitForTimeout(250);}
const menu=p.locator('.conversation-manager .row-actions');await menu.locator('summary').click();
const deletion=menu.locator('.row-action-buttons > .danger');await deletion.click();
const confirmation=p.locator('dialog:modal').filter({has:p.locator('.confirmation')});await confirmation.waitFor();
await p.waitForTimeout(250);await p.screenshot({path:require('./test_environment.cjs').artifactPath(`nola-web-layers/confirmation-${width}-${height}.png`)});
const cancel=confirmation.locator('button').first();assert.equal(await cancel.evaluate(e=>{let r=e.getBoundingClientRect();return r.top>=0&&r.bottom<=innerHeight}),true,'confirmation cancel reachable');
await p.keyboard.press('Escape');await confirmation.waitFor({state:'hidden'});await p.waitForFunction(()=>document.activeElement?.matches('.row-action-buttons > .danger'));assert.equal(await deletion.evaluate(e=>e===document.activeElement),true,'nested modal restores delete opener');
await p.keyboard.press('Escape');assert.equal(await menu.evaluate(e=>e.open),false);assert.equal(await menu.locator('summary').evaluate(e=>e===document.activeElement),true);
if(width<=760){await p.keyboard.press('Escape');assert.equal(await p.locator('dialog:modal').count(),0);}
await p.locator('#chat-composer').fill('保留草稿');await p.locator('#chat-composer').scrollIntoViewIfNeeded();assert.equal(await p.locator('#chat-composer').evaluate(e=>{let r=e.getBoundingClientRect();return r.top>=0&&r.bottom<=innerHeight}),true,'short viewport composer reachable');
await p.close();}}finally{await b.close();}})().catch(e=>{console.error(e);process.exitCode=1});
