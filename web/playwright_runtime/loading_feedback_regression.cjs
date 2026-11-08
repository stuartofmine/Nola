// Delayed reads and failed retries; isolated fixture only, no Host writes.
const {chromium}=require('playwright-core');const assert=require('node:assert/strict');
(async()=>{const browser=await chromium.launch({executablePath:'/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',headless:true});try{
 const page=await browser.newPage({viewport:{width:390,height:900}});const base=process.env.UI_FIXTURE_URL||'http://127.0.0.1:5139';
 await page.route('**/api/web/session',r=>r.fulfill({json:{authenticated:true,localTrust:true,hostId:'ui-review-fixture'}}));
 let release,fail=true,requests=0;
 await page.route('**/api/agent/settings',async r=>{requests++;await new Promise(resolve=>release=resolve);await r.fulfill(fail?{status:503,json:{error:'fixture_failure'}}:{json:{skills:[],mcp_servers:[]}});});
 await page.goto(`${base}/capabilities`);await page.locator('#capability-panel [role=status]').waitFor();release();
 const retry=page.locator('.notice.error button');await retry.waitFor();await retry.focus();await page.keyboard.press('Enter');
 await page.waitForFunction(()=>document.querySelector('#capability-panel').getAttribute('aria-busy')==='true');
 assert.equal(await retry.evaluate(e=>document.activeElement===e),true,'retry focus retained during read');
 const pendingCount=requests;await retry.press('Enter');await page.waitForTimeout(80);assert.equal(requests,pendingCount,'aria-disabled retry blocks duplicate reads');
 release();await page.waitForFunction(()=>document.querySelector('.notice.error button').getAttribute('aria-disabled')==='false');
 assert.equal(await retry.evaluate(e=>document.activeElement===e),true,'failed retry retains focus');
 fail=false;await retry.press('Enter');await page.waitForFunction(()=>document.querySelector('#capability-panel').getAttribute('aria-busy')==='true');release();
 await page.waitForFunction(()=>document.activeElement?.id==='capability-panel');
 fail=true;await page.reload();await page.locator('#capability-panel [role=status]').waitFor();release();await retry.waitFor();
 await retry.click();await page.locator('.management-content input[type=search]').focus();fail=false;release();
 await page.waitForFunction(()=>document.querySelector('#capability-panel').getAttribute('aria-busy')==='false');
 assert.equal(await page.locator('.management-content input[type=search]').evaluate(e=>e===document.activeElement),true);
 await page.route('**/api/data-sources',r=>r.fulfill({json:{sources:[]}}));
 await page.route('**/api/data-sources/usage',async r=>{await new Promise(resolve=>release=resolve);await r.fulfill({json:{usageRecords:[]}});});
 await page.goto(`${base}/data-sources`);await page.locator('#data-tab-usage').click();await page.locator('#data-panel-usage [role=status]').waitFor();
 assert.equal(await page.locator('#data-panel-usage').getAttribute('aria-busy'),'true');release();await page.waitForFunction(()=>document.querySelector('#data-panel-usage').getAttribute('aria-busy')==='false');
 console.log('PASS delayed usage loading, failed retry focus retention, successful retry result focus, no response focus stealing');
}finally{await browser.close();}})().catch(e=>{console.error(e);process.exitCode=1});
