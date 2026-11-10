const{chromium}=require('playwright-core');const assert=require('node:assert/strict');
(async()=>{const b=await chromium.launch({...require('./test_environment.cjs').browserOptions(),headless:true});try{for(const width of [320,768,1440]){
let previewLoads=0;const p=await b.newPage({viewport:{width,height:400}});const base=process.env.UI_FIXTURE_URL||'http://127.0.0.1:5139';
await p.route('**/api/chat/conversations/review/browser',r=>r.fulfill({json:{running:true,pages:[{index:0,url:'https://example.test',title:'Isolated browser',active:true}],currentUrl:'https://example.test',previewUrl:'http://127.0.0.1:59999/fixture-preview'}}));
await p.route('http://127.0.0.1:59999/fixture-preview',r=>{previewLoads++;return r.fulfill({contentType:'text/html',body:'<!doctype html><button id="inside">Preview focus target</button>'});});
await p.goto(`${base}/chat?conversation=review`);await p.locator('#chat-composer').waitFor();await p.waitForTimeout(250);if(width<=760){await p.locator('.chat-header [data-panel=settings]').click();await p.locator('dialog:modal .panel-toolbar [data-panel=resources]').click();}else await p.locator('.chat-header [data-panel=resources]').click();
const toggle=p.locator('.preview-actions button');await toggle.scrollIntoViewIfNeeded();await p.frameLocator('.preview-shell iframe').locator('#inside').evaluate(e=>e.dataset.retained='yes');await toggle.click();await p.waitForTimeout(250);
const expanded=p.locator('.preview-shell:modal');assert.equal(await expanded.count(),1);assert.equal(await toggle.evaluate(e=>e===document.activeElement),true);
assert.equal(await toggle.evaluate(e=>{let r=e.getBoundingClientRect();return e.contains(document.elementFromPoint(r.x+r.width/2,r.y+r.height/2))}),true,'top-layer preview controls own hit test');
if(width===768){await p.setViewportSize({width:1440,height:400});await p.waitForTimeout(250);assert.equal(await toggle.evaluate(e=>{let r=e.getBoundingClientRect();return e.contains(document.elementFromPoint(r.x+r.width/2,r.y+r.height/2))}),true,'preview remains top layer after parent breakpoint');}
await p.screenshot({path:require('./test_environment.cjs').artifactPath(`nola-web-layers/preview-${width}.png`)});
await p.frameLocator('.preview-shell iframe').locator('#inside').focus();await p.keyboard.press('Escape');await p.waitForTimeout(100);
console.log(width,'cross-origin iframe Escape does not bubble; verifying Shift+Tab exit');
if(await expanded.count()){await p.keyboard.press('Shift+Tab');assert.equal(await toggle.evaluate(e=>e===document.activeElement),true,'iframe keyboard exit returns to collapse control');await p.keyboard.press('Escape');}
assert.equal(await p.locator('.preview-shell:modal').count(),0);assert.equal(await toggle.evaluate(e=>e===document.activeElement),true);
assert.equal(await p.frameLocator('.preview-shell iframe').locator('#inside').getAttribute('data-retained'),'yes','same embedded viewer state retained');assert.equal(previewLoads,1,'expansion does not reload iframe');
console.log('PASS preview',width,'top layer, breakpoint preservation, keyboard exit, one iframe load and retained state');await p.close();}}finally{await b.close();}})().catch(e=>{console.error(e);process.exitCode=1});
