// Actual routes with in-memory API responses; no request reaches a user Host.
const {chromium} = require('playwright-core');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const base = process.env.UI_FIXTURE_URL || 'http://127.0.0.1:5139';
const out = process.env.UI_EVIDENCE_DIR || '/tmp/nola-web-full-ui';
const longName = '产品研究与长期资料管理 · 多语言长名称 ProductResearchWorkspace_2026';
const spec = {kind:'agent',spec:{prompt:'检查工作区的待办并整理本周摘要。',execution_scope:{kind:'local_workspace',root:'/fixtures/project'},permission_profile:'cautious',model_selection:'use_current_default_at_admission',context_refs:[],skill_refs:[]}};
const actions = [0,1].map(i => ({id:'fixture-'+i,name:i ? '每周资料检查' : longName,url:'/fixtures/project',format:'agent',userPrompt:'整理待办',revision:1,frequency:'daily',weekday:null,time:'09:00',timezone:'local',enabled:!i,updatedAt:'2026-10-20T10:00:00Z',scriptApproved:null,scriptCwd:null,latestResult:null,executionSpec:spec}));
(async()=>{
 fs.mkdirSync(out,{recursive:true});
 const browser = await chromium.launch({executablePath: process.env.CHROME_PATH || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',headless:true});
 try {
  for(const colorScheme of ['light','dark']) {
   const page=await browser.newPage({viewport:{width:1440,height:1000},colorScheme});
   const errors=[]; page.on('pageerror',e=>errors.push(e.message));
   await page.route('**/api/**', async route=>{
    const path=new URL(route.request().url()).pathname;
    const json = path==='/api/web/session' ? {authenticated:true,sessionAuthenticated:false,localTrust:true,hostId:'ui-review-fixture'}
     : path==='/api/data-sources' ? {sources:[{source:{id:'source-1',name:longName,kind:'localFile',location:'/fixtures/project/records-with-long-file-name.json',accessPolicy:'conversationOptIn',validationContract:null}}]}
     : path==='/api/data-sources/usage' ? {usageRecords:[]}
     : path==='/api/scheduled-actions' ? {actions,executionCapabilities:{manualRun:true,scheduledDispatch:false,promptAutomations:true}}
     : path==='/api/agent/settings' ? {skills:[{name:longName,description:'将工作区资料整理成可追溯的简报；授权仍按实际调用判断。',content:'Read the project documents.',enabled:true,source:'custom'},{name:'代码阅读',description:'阅读结构与测试，保留现有数据。',content:'Read code.',enabled:false,source:'custom'}],mcp_servers:[]}
     : path==='/api/skills' ? {skills:[]}
     : undefined;
    if(json) return route.fulfill({json});
    return route.fallback();
   });
   for(const route of ['settings','data-sources','scheduled-actions','capabilities']) {
    await page.goto(`${base}/${route}`); await page.locator('.management-content').waitFor();
    await page.waitForTimeout(150);
    for(const width of [1440,390,320]) {
     await page.setViewportSize({width,height:width<500?900:1000});
     await page.evaluate(()=>new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r))));
     assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),true,`${route} ${colorScheme} ${width} overflow`);
     await page.screenshot({path:`${out}/${route}-${colorScheme}-${width}.png`,fullPage:true});
    }
    if(route==='settings') {
     const tab=page.locator('#settings-tab-general'); await tab.focus(); await page.keyboard.press('End');
     assert.equal(await page.locator('#settings-tab-diagnostics').getAttribute('aria-selected'),'true');
     const endBounds=await page.locator('#settings-tab-diagnostics').boundingBox(); assert.ok(endBounds.x>=0 && endBounds.x+endBounds.width<=320);
     await page.screenshot({path:`${out}/settings-tabs-end-${colorScheme}-320.png`});
     await page.keyboard.press('Home'); assert.equal(await tab.getAttribute('aria-selected'),'true');
     const bounds=await tab.boundingBox(); assert.ok(bounds.x>=0 && bounds.x+bounds.width<=320);
     await page.emulateMedia({reducedMotion:'reduce'});
     assert.ok(await page.locator('#settings-panel-general').evaluate(e=>parseFloat(getComputedStyle(e).animationDuration)<=.00001));
     await page.emulateMedia({reducedMotion:'no-preference'});
    }
    if(route==='data-sources') {
     await page.locator('#data-tab-sources').click();
     await page.screenshot({path:`${out}/sources-${colorScheme}-320.png`,fullPage:true});
     await page.locator('#data-panel-sources .actions button').first().click();
     await page.locator('dialog:modal').waitFor();
     assert.equal(await page.locator('dialog:modal').evaluate(e=>e.contains(document.activeElement)),true);
     await page.screenshot({path:`${out}/source-editor-${colorScheme}-320.png`});
     await page.keyboard.press('Escape');
    }
    if(route==='scheduled-actions') {
     const menu=page.locator('.automation-row .action-disclosure').first();
     await menu.locator('summary').focus(); await page.keyboard.press('Enter');
     assert.equal(await menu.locator('.danger').isVisible(),true);
     await page.keyboard.press('Escape'); assert.equal(await menu.evaluate(e=>e.open),false);
     await page.locator('.automation-row .heading > button').first().click();
     await page.locator('form.card input').first().waitFor();
     assert.equal(await page.locator('form.card input').first().evaluate(e=>e===document.activeElement),true);
     assert.equal(await page.locator('.schedule-fields').getAttribute('disabled')!==null,true);
     await page.screenshot({path:`${out}/automation-editor-${colorScheme}-320.png`,fullPage:true});
     // This context is disposable; no form submission or Host mutation is made.
    }
    if(route==='capabilities') {
     await page.locator('.capability').first().focus(); await page.keyboard.press('Enter');
     await page.locator('dialog:modal').waitFor(); await page.screenshot({path:`${out}/capability-editor-${colorScheme}-320.png`});
     await page.keyboard.press('Escape');
    }
    // Large text at a regular viewport checks rem-based typography and wrap.
    await page.setViewportSize({width:1280,height:1000});
    await page.evaluate(()=>document.documentElement.style.fontSize='200%');
    assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),true,`${route} 200% text overflow`);
    await page.screenshot({path:`${out}/${route}-${colorScheme}-text200.png`,fullPage:true});
    await page.evaluate(()=>document.documentElement.style.fontSize='');
   }
   await page.goto(`${base}/chat?conversation=review`);
   await page.locator('#chat-composer').waitFor();
   await page.locator('#chat-composer').fill('保留正在编辑的草稿');
   await page.setViewportSize({width:1440,height:1000});
   const settingsToggle=page.locator('.chat-header [aria-controls="chat-details"]').last();
   await settingsToggle.focus(); await page.keyboard.press('Enter');
   await page.locator('.session-settings').waitFor(); await page.waitForTimeout(220);
   await page.screenshot({path:`${out}/chat-inspector-${colorScheme}-1440.png`});
   await page.setViewportSize({width:320,height:900});
   await page.locator('dialog:modal').waitFor();
   await page.waitForTimeout(250);
   assert.equal(await page.locator('dialog:modal .panel-toolbar > button').evaluate(e=>e===document.activeElement && e.getClientRects().length>0),true);
   assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),true);
   await page.screenshot({path:`${out}/chat-inspector-${colorScheme}-320.png`});
   await page.keyboard.press('Escape');
   assert.equal(await page.locator('#chat-composer').inputValue(),'保留正在编辑的草稿');
   await page.locator('#chat-composer').fill('');
   let failSource=true;
   await page.route('**/api/data-sources',route=>failSource ? route.fulfill({status:503,json:{error:'fixture_sources_unavailable'}}) : route.fallback());
   await page.goto(`${base}/data-sources`);
   await page.locator('#data-panel-overview [role="alert"]').waitFor();
   await page.screenshot({path:`${out}/sources-error-${colorScheme}-320.png`});
   failSource=false;
   await page.locator('#data-panel-overview .row button').first().click();
   await page.locator('.source-stats').waitFor();
   assert.equal(await page.locator('#data-panel-overview [role="alert"]').count(),0);
   assert.deepEqual(errors,[]); console.log(`PASS ${colorScheme}: 4 real management routes, desktop/390/320/200% text, tab keyboard, editor focus, menus and reduced motion`);
   await page.close();
  }
 } finally {await browser.close();}
})().catch(e=>{console.error(e);process.exitCode=1});
