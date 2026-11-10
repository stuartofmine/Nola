// Fault only queue reads; history and Surface reads use the actual isolated Host.
const {chromium}=require('playwright-core');const fs=require('node:fs');const assert=require('node:assert/strict');
(async()=>{
 const state=JSON.parse(fs.readFileSync((process.env.NOLA_A2UI_HOST_STATE || require('./test_environment.cjs').artifactPath('nola-a2ui-real-host-state.json')),'utf8'));
 const cid=process.env.NOLA_A2UI_TEST_CONVERSATION;assert.ok(cid);assert.equal(new URL(state.url).hostname,'127.0.0.1');
 const browser=await chromium.launch({...require('./test_environment.cjs').browserOptions(),headless:true});
 const results=[];
 try{for(const status of [409,503]){
  const page=await browser.newPage();let queueReads=0;let sends=0;
  page.on('request',r=>{if(new URL(r.url()).pathname.endsWith('/agent/v2/stream'))sends++;});
  await page.route('**/queued-input?*',route=>{queueReads++;return route.fulfill({status,json:{error:'isolated queue read failure'}});});
  await page.goto(state.url,{waitUntil:'domcontentloaded'});
  await page.goto(`${state.url}/chat?conversation=${encodeURIComponent(cid)}`,{waitUntil:'domcontentloaded'});
  await page.locator('.surface-reference').first().waitFor({timeout:30000});
  assert.ok(await page.locator('article[data-message-id]').count()>0,'queue failure must not prevent history rendering');
  assert.ok(queueReads>0);assert.equal(sends,0);
  results.push({status,history_visible:true,queueReads,sends});await page.close();
 }
 fs.writeFileSync(require('./test_environment.cjs').artifactPath('nola-a2ui-queue-read-isolation.json'),JSON.stringify({passed:true,results},null,2));console.log('PASS queue failure leaves canonical history readable');
 }finally{await browser.close();}
})().catch(e=>{console.error(e.name,e.message);process.exitCode=1;});
