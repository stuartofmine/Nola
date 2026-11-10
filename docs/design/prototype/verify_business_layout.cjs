// SPDX-License-Identifier: Apache-2.0
// Standalone prototype layout, entry links, keyboard recovery and clipboard refusal.
const {chromium}=require('../../../apps/web/playwright_runtime/node_modules/playwright-core');
const fs=require('node:fs'),assert=require('node:assert/strict');
const base=(process.env.PROTOTYPE_URL||'http://127.0.0.1:8071').replace(/\/$/,'');
const out=process.env.PROTOTYPE_OUT||require('../../../apps/web/playwright_runtime/test_environment.cjs').artifactPath('nola-prototype-business-layout');fs.mkdirSync(out,{recursive:true});
(async()=>{
const browser=await chromium.launch({...require('../../../apps/web/playwright_runtime/test_environment.cjs').browserOptions(),headless:true});
const context=await browser.newContext({reducedMotion:'reduce'}), page=await context.newPage();const errors=[];page.on('pageerror',e=>errors.push(e.message));
await context.route('**/*',r=>new URL(r.request().url()).origin===new URL(base).origin?r.continue():r.abort());
let layouts=0;const scenes=['permission','host-pair','inspector','mcp-config','compose','general-fonts'];
for(const width of [700,701,768,1000,1920])for(const theme of ['light','dark'])for(const scene of scenes){
await page.setViewportSize({width,height:1000});await page.goto(`${base}/scenarios.html?scene=${scene}&platform=mac&theme=${theme}`);await page.locator('#device[data-scene]').waitFor();assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1));layouts++;
}
for(const platform of ['mac','ios','android','web','windows','cli'])for(const theme of ['light','dark']){
await page.setViewportSize({width:['ios','android'].includes(platform)?390:1440,height:1000});await page.goto(`${base}/scenarios.html?scene=long-run&state=1&platform=${platform}&theme=${theme}`);await page.locator('#device[data-scene]').waitFor();await page.locator('#device').screenshot({path:`${out}/${platform}-${theme}-long-task-platform.png`});
}
await page.goto(base+'/scenarios.html?scene=privacy-export&state=2&platform=mac');await page.locator('[data-transition="0"]').click();await page.locator('#confirm[open]').waitFor();assert.equal(await page.evaluate(()=>document.activeElement.id),'confirm-cancel');await page.keyboard.press('Escape');assert.equal(await page.evaluate(()=>document.activeElement.dataset.transition),'0');
await page.goto(base+'/');await page.getByRole('button',{name:'设置',exact:true}).last().click();await page.locator('.prototype-state-controls>summary').click();await page.getByRole('link',{name:'全业务场景与状态总览 ↗',exact:true}).click();await page.locator('#device[data-scene]').waitFor();
await page.setViewportSize({width:390,height:900});await page.goto(base+'/');await page.locator('[data-action="im:nav:settings"]').click();await page.locator('.im-preview-tools>summary').click();await page.getByRole('link',{name:'全业务场景与状态总览 ↗',exact:true}).click();await page.locator('#device[data-scene]').waitFor();assert.equal(new URL(page.url()).searchParams.get('platform'),'ios');
await context.addInitScript(()=>Object.defineProperty(navigator,'clipboard',{configurable:true,value:{writeText:()=>Promise.reject(new Error('fixture-denied'))}}));
await page.goto(base+'/scenarios.html?scene=compose&state=2&platform=ios');await page.locator('[data-copy]').click();await page.getByRole('textbox',{name:'手动复制回复',exact:true}).waitFor();assert.equal(await page.getByRole('textbox',{name:'手动复制回复',exact:true}).getAttribute('readonly'),'');
assert.equal(errors.length,0);fs.writeFileSync(`${out}/receipt.json`,JSON.stringify({passed:true,layouts,platformCaptures:12,checks:['confirmation escape and focus restoration','desktop and mobile entry links','clipboard refusal fallback'],errors}));await browser.close();console.log('PASS extra layout / entry / keyboard / clipboard checks');
})().catch(e=>{console.error(e);process.exitCode=1;});
