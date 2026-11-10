// SPDX-License-Identifier: Apache-2.0
// Real Chromium / production components; isolated API fixtures, no provider calls.
const {chromium}=require('playwright-core');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const base=process.env.UI_FIXTURE_URL||'http://127.0.0.1:5139';
const out=process.env.UI_EVIDENCE_DIR||require('./test_environment.cjs').artifactPath('nola-page-matrix');
const routes=['today','conversations','chat','activity','results','dashboard','data-sources','scheduled-actions','models','model-test','capabilities','workspace-review','settings','login'];
const catalog=fs.readFileSync(require('node:path').join(__dirname,'../frontend/src/lib/pageRoutes.ts'),'utf8');
assert.deepEqual([...catalog.matchAll(/'\/([^']+)':/g)].map(m=>m[1]).sort(),[...routes].sort(),'Every routed page must be included in the matrix');
const categories=['general','providers','agent','connections','data','diagnostics'];
let invalidCapabilities=false, invalidSave=false, writes=0;
const name='跨语言长期研究与工作区资料 ProductResearchWorkspace_2026';
async function fixture(route){
 const p=new URL(route.request().url()).pathname;
 const known={
 '/api/web/session':{authenticated:true,sessionAuthenticated:false,localTrust:true,hostId:'page-matrix'},
 '/api/chat/conversations':{conversations:[]},
 '/api/chat/agent/v2/observe':{source_host_id:'page-matrix',result:null},
 '/api/model-profiles':{profiles:[]},
 '/api/local-models':{models:[],binaryAvailable:false},
 '/api/data-sources':{sources:[{source:{id:'matrix-source',name,kind:'localFile',location:'/fixtures/长期研究资料.json',accessPolicy:'conversationOptIn',validationContract:null}}]},
 '/api/data-sources/usage':{usageRecords:[]},
 '/api/scheduled-actions':{actions:[],executionCapabilities:{manualRun:true,scheduledDispatch:false,promptAutomations:true}},
 '/api/agent/settings':{skill_settings_backup_key:null,skills:[{name,description:'阅读与整理资料，保留授权边界。',content:'Read documents.',enabled:true,source:'custom'}],mcp_servers:[],execution_backends:[]},
 '/api/skills':{skills:[]},
 '/api/workspace/today':{source_host_id:'page-matrix',snapshot:{needs_attention:false,unrepresented_expert_contexts:[],conversations:[],recent_conversations:[],queued_instances:[],has_more_conversations:false,next_conversation_offset:null,has_more_queued_instances:false}},
 };
 if(p==='/api/agent/settings' && route.request().method()==='POST') { writes++; if(invalidSave)return route.fulfill({json:{skills:[],mcp_servers:[]}}); }
 if(p==='/api/agent/settings' && invalidCapabilities)return route.fulfill({json:{skills:[],mcp_servers:[]}});
 if(Object.hasOwn(known,p))return route.fulfill({json:known[p]});
 return route.fulfill({status:503,json:{code:'isolated_page_matrix_unavailable'}});
}
(async()=>{
 fs.mkdirSync(out,{recursive:true});
 const browser=await chromium.launch({...require('./test_environment.cjs').browserOptions(),headless:true});
 const report={kind:'isolated-components-no-model',views:[],issues:[],errors:[]};
 try{
 for(const scheme of ['light','dark'])for(const width of [320,390,1440]){
  const context=await browser.newContext({viewport:{width,height:900},colorScheme:scheme,locale:'zh-CN',reducedMotion:process.env.UI_AUDIT_MOTION==='1'?'no-preference':'reduce'});
  await context.route('**/*',r=>new URL(r.request().url()).origin!==base?r.abort():new URL(r.request().url()).pathname.startsWith('/api/')?fixture(r):r.continue());
  await context.routeWebSocket('**/*',s=>s.close());
  await context.addInitScript(() => {
   let enabled=false, reads=0;
   const labels={title:'启动设置',body:'登录 Windows 时启动 Nola',enabled:'已启用',disabled:'已停用',unavailable:'不可用'};
   window.NolaDesktop={platform:'windows',loginItem:async()=>{if(++reads===1)throw new Error('fixture disconnected');return {available:true,enabled,labels};},setLoginItem:async value=>{enabled=value;throw new Error('fixture reply lost after applying');}};
  });
  const page=await context.newPage(); page.setDefaultTimeout(10000);
  page.on('pageerror',e=>report.errors.push({width,scheme,url:page.url(),message:e.message}));
  async function capture(label){
   await page.evaluate(()=>new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r))));
   const result=await page.evaluate(()=>{
    const visible=e=>!!e.getClientRects().length && getComputedStyle(e).visibility!=='hidden' && !e.closest('[inert]');
    const controls=[...document.querySelectorAll('button,a,input,select,textarea,summary')].filter(visible);
    const unnamed=controls.filter(e=>!e.textContent.trim()&&!e.getAttribute('aria-label')&&!e.getAttribute('aria-labelledby')&&!e.getAttribute('title')&&!e.labels?.length&&!e.getAttribute('placeholder')).map(e=>e.outerHTML.slice(0,180));
    const clipped=controls.filter(e=>e.tagName==='BUTTON'&&e.scrollWidth>e.clientWidth+2).map(e=>e.textContent.trim());
    return{overflow:document.documentElement.scrollWidth>innerWidth+1,controls:controls.length,unnamed,clipped,title:document.title,alerts:[...document.querySelectorAll('[role=alert]')].filter(visible).map(e=>e.textContent.trim())};
   });
   report.views.push({label,width,scheme,...result});
   if(result.overflow||result.unnamed.length||result.clipped.length)report.issues.push({label,width,scheme,...result});
   await page.screenshot({path:`${out}/${label}-${width}-${scheme}.png`});
  }
  for(const route of routes){
   await page.goto(`${base}/${route}`,{waitUntil:'networkidle'});
   if(process.env.UI_AUDIT_MOTION==='1' && route==='today') {
    await page.evaluate(()=>{const button=document.createElement('button');button.id='audit-press-probe';button.type='button';button.textContent='操作反馈';button.style.cssText='position:fixed;right:12px;bottom:100px;z-index:100';document.body.append(button);});
    const control=page.locator('#audit-press-probe'),before=await control.boundingBox();
    await control.hover();await page.mouse.down();await page.waitForTimeout(60);
    const pressed=await control.boundingBox();
    assert.ok(Math.abs(before.width-pressed.width)<.01 && Math.abs(before.height-pressed.height)<.01,'Press feedback must preserve glyph and control geometry');
    await page.mouse.up();await control.evaluate(node=>node.remove());
   }
   if(process.env.UI_AUDIT_MOTION==='1' && route==='today' && width<=700) {
    let release, intercepted;
    const gate=new Promise(resolve=>release=resolve), requested=new Promise(resolve=>intercepted=resolve);
    const hold=async route=>{intercepted();await gate;await route.continue();};
    await page.route('**/ui/assets/Activity-*.js',hold);
    try {
     await page.locator('.mobile-primary-navigation a[href="/activity"]').click();
     await Promise.race([requested,new Promise((_,reject)=>setTimeout(()=>reject(new Error('Expected isolated Activity module request')),5000))]);
     assert.equal(await page.evaluate(()=>document.activeElement?.matches('#page-main,.workspace-content')),false,'Loading must not steal focus before the selected page exists');
     release();
     await page.waitForFunction(()=>document.activeElement?.id==='page-main');
    } finally {release();await page.unroute('**/ui/assets/Activity-*.js',hold);}
    await page.goto(`${base}/today`,{waitUntil:'networkidle'});
   }
   await capture(route);
   if(process.env.UI_AUDIT_MOTION==='1' && route==='workspace-review') {
    assert.equal(await page.locator('.workspace-review [role=alert]').count(),0,'No conversation is an actionable empty state, not a read failure');
    assert.ok(await page.locator('.workspace-review a[href="/conversations"]').count());
   }
   if(process.env.UI_AUDIT_MOTION==='1' && route==='chat' && width<=700) {
    assert.equal(new URL(page.url()).pathname,'/conversations','Direct and in-app chat entry must agree on phones');
   }
   if(route==='data-sources') {
    for(const tab of width<=700?['knowledge','sources','memory','plans','activity']:['sources','memory','plans','usage']) {
     await page.goto(`${base}/data-sources?tab=${tab}`,{waitUntil:'networkidle'});
     await capture(`information-${tab}`);
     if(tab==='sources') {
      const add=page.locator('.page-actions .primary:visible');await add.click();
      const dialog=page.locator('dialog:modal');await dialog.waitFor();
      assert.equal(await dialog.evaluate(e=>e.contains(document.activeElement)),true,'source editor focus');
      await capture('source-editor');await page.keyboard.press('Escape');await dialog.waitFor({state:'hidden'});
      assert.equal(await add.evaluate(e=>e===document.activeElement),true,'source editor restores opener');
     }
    }
   }
   if(route==='capabilities') {
    const card=page.locator('button.capability:visible').first();await card.click();
    const dialog=page.locator('dialog:modal');await dialog.waitFor();
    assert.equal(await dialog.evaluate(e=>e.contains(document.activeElement)),true,'capability editor focus');
    await capture('capability-editor');await page.keyboard.press('Escape');await dialog.waitFor({state:'hidden'});
    assert.equal(await card.evaluate(e=>e===document.activeElement),true,'capability editor restores opener');
    invalidCapabilities=true;
    await page.reload({waitUntil:'networkidle'});
    const alert=page.locator('[role=alert]:visible');await alert.waitFor();
    await capture('capability-malformed-recovery');invalidCapabilities=false;
    await alert.getByRole('button').click();await page.locator('button.capability:visible').first().waitFor();
    assert.equal(await page.locator('[role=alert]:visible').count(),0,'valid catalog retry recovers');
    await page.locator('button.capability:visible').first().click();
    const editor=page.locator('dialog:modal'), field=editor.locator('input:not([type=checkbox])').first();
    const original=await field.inputValue(), previousWrites=writes;
    await field.fill('未确认保存的独立草稿 Unconfirmed draft');invalidSave=true;
    await editor.locator('form .actions button.primary').click();
    await editor.locator('[role=alert]').waitFor();
    assert.equal(await field.inputValue(),'未确认保存的独立草稿 Unconfirmed draft');
    assert.equal(writes,previousWrites+1,'malformed save must never auto-replay');
    await capture('capability-unconfirmed-save');invalidSave=false;
    await field.fill(original);await page.keyboard.press('Escape');await editor.waitFor({state:'hidden'});
    await page.locator('[role=tab][id$="capability-mcp"]').click();await capture('capabilities-mcp');
   }
   if(route==='settings') {
    await page.goto(`${base}/settings?section=notifications`,{waitUntil:'networkidle'});
    const outer=new URL(page.url()), destination=new URL(outer.searchParams.get('content')||outer.href,base);
    assert.equal(destination.searchParams.get('section'),'general','Existing notification links open General');
    assert.ok(await page.locator('[id$=settings-panel-general] [id$=settings-notifications]:visible').count());
    assert.equal(await page.locator('[role=tab][id$=settings-tab-notifications]').count(),0,'Notifications have one home in General');
    // Native settings must mount after SPA navigation, expose a failed read,
    // and reconcile a lost write response without replaying the mutation.
    await page.goto(`${base}/settings?section=general`,{waitUntil:'networkidle'});
    const card=page.locator('#desktopLoginItemCard');
    if(await card.count()) {
     await card.locator('[role=alert]').waitFor();
     await card.getByRole('button').click();
     const toggle=card.locator('button[aria-pressed]');await toggle.waitFor();
     assert.equal(await toggle.getAttribute('aria-pressed'),'false');
     await toggle.click();
     await page.waitForFunction(()=>document.querySelector('#desktopLoginItemCard button[aria-pressed]')?.getAttribute('aria-pressed')==='true');
     assert.equal(await card.locator('[role=alert]').count(),0,'lost write reconciles actual saved value');
    }
    await page.goto(`${base}/settings`,{waitUntil:'networkidle'});
    await page.locator('.settings-overview:visible,[role=tab][id$="settings-tab-general"]:visible,.settings-back button:visible').first().waitFor();
    const back=page.locator('.settings-back button:visible');if(await back.count())await back.click();
   }
   if(route==='settings')for(const category of categories){
    // Scope rewrites embedded IDs; target semantic links, never stale global IDs.
    if(await page.locator('.settings-overview:visible').count())await page.locator(`.settings-overview a[href="/settings?section=${category}"]`).click();
    else await page.locator(`[role=tab][id$="settings-tab-${category}"]:visible`).click();
    await page.waitForLoadState('networkidle');await capture(`settings-${category}`);
    const back=page.locator('.settings-back button:visible');if(await back.count())await back.click();
   }
  }
  await context.close();
 }
 }finally{fs.writeFileSync(`${out}/receipt.json`,JSON.stringify(report,null,2));await browser.close();}
 assert.deepEqual(report.errors,[],'uncaught browser errors');
 assert.deepEqual(report.issues,[],'page layout / accessible-name failures');
 console.log(`PASS ${report.views.length} page/section layouts across three widths and both themes`);
})().catch(e=>{console.error(e);process.exitCode=1});
