// SPDX-License-Identifier: Apache-2.0
// Chromium interaction acceptance for the standalone prototype; no app or model calls.
const {chromium}=require('../../../apps/web/playwright_runtime/node_modules/playwright-core');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const base=process.env.PROTOTYPE_URL||'http://127.0.0.1:8067';
const out=process.env.PROTOTYPE_OUT||require('../../../apps/web/playwright_runtime/test_environment.cjs').artifactPath('nola-prototype-sync-20261110');
(async()=>{
 fs.mkdirSync(out,{recursive:true});
 const browser=await chromium.launch({...require('../../../apps/web/playwright_runtime/test_environment.cjs').browserOptions(),headless:true});
 const report={kind:'prototype-only',screens:[],cases:[],errors:[],externalRequests:[]};
 try{
 for(const [width,scheme,os] of [[320,'light','ios'],[320,'dark','ios'],[390,'light','ios'],[390,'dark','ios'],[390,'light','android'],[390,'dark','android'],[1000,'dark','desktop'],[1440,'light','desktop'],[1440,'dark','desktop']]){
  const context=await browser.newContext({viewport:{width,height:900},colorScheme:scheme,reducedMotion:'reduce',permissions:['clipboard-read','clipboard-write']});
  const page=await context.newPage();page.setDefaultTimeout(8000);
  await context.route('**/*',r=>new URL(r.request().url()).origin===base?r.continue():(report.externalRequests.push(r.request().url()),r.abort()));
  await page.addInitScript(theme=>localStorage.setItem('nola-prototype-theme',theme),scheme);
  page.on('pageerror',e=>report.errors.push({width,scheme,os,message:e.message}));
  await page.goto(base);
  const click=action=>page.locator(`[data-action="${action}"]:visible`).last().click();
  async function shot(name){
   await page.waitForTimeout(80);
   await page.locator('#toast.visible').waitFor({state:'hidden'});
   assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),true,`${name} page overflow`);
   const file=`${os}-${width}-${scheme}-${name}.png`;await page.screenshot({path:`${out}/${file}`});report.screens.push(file);
  }
  async function settings(){if(width<=700)await click('im:nav:settings');else {await page.getByRole('button',{name:'设置',exact:true}).last().click();const back=page.locator('[data-action="settings-back"]:visible');if(await back.count())await back.click();}}
  await settings();
  if(os==='android'){await page.locator('.im-preview-tools>summary').click();await click('im:platform');}
  await shot('settings');
  if(width<=700){
   assert.equal(await page.locator('.im-screen > .im-group > .im-setting-row').count(),os==='android'?4:5);
   for(const removed of ['im:nav:im-appearance','im:nav:im-updates','im:nav:memory','im:nav:plans','im:nav:data'])assert.equal(await page.locator(`[data-action="${removed}"]:visible`).count(),0);
  }else assert.equal(await page.locator('[data-action^="ex-settings:"]:visible').count(),6);
  if(width>700){
   const row=page.locator('[data-action="ex-settings:general"]');const before=await row.evaluate(e=>getComputedStyle(e).backgroundColor);
   await row.hover();await page.waitForTimeout(150);const after=await row.evaluate(e=>getComputedStyle(e).backgroundColor);assert.notEqual(after,before,'desktop hover must visibly change in both themes');
   await row.click();
  }else await click('im:nav:im-general');
  await page.getByRole('heading',{name:'通用',exact:true}).waitFor();
  if(width<=700 && os==='ios'){
   const toggle=page.getByRole('switch',{name:'启用通知（演示）',exact:true});
   const before=await toggle.getAttribute('aria-checked');await toggle.click();
   assert.notEqual(await page.getByRole('switch',{name:'启用通知（演示）',exact:true}).getAttribute('aria-checked'),before);
   assert.equal(await page.locator('dialog:modal').count(),0,'General preferences edit inline');
   assert.equal(await page.evaluate(()=>document.activeElement?.dataset.action),'im:notification-toggle');
  }
  await shot('general');
  if(width>700){
   const startup=page.getByRole('switch',{name:'登录时启动 Nola（演示）',exact:true});
   await startup.scrollIntoViewIfNeeded();
   const before=await startup.getAttribute('aria-checked'),offset=await page.locator('.studio-center').evaluate(e=>e.scrollTop);
   await startup.focus();await page.keyboard.press('Space');
   assert.notEqual(await page.getByRole('switch',{name:'登录时启动 Nola（演示）',exact:true}).getAttribute('aria-checked'),before);
   assert.equal(await page.evaluate(()=>document.activeElement?.dataset.action),'ex-pref:startup:general');
   assert.ok(Math.abs(await page.locator('.studio-center').evaluate(e=>e.scrollTop)-offset)<2,'Settings toggle preserves reading position');
   await shot('general-lower');
   const padding=await page.locator('.prototype-general>.inline-card').first().evaluate(e=>parseFloat(getComputedStyle(e).paddingLeft));
   assert.ok(padding>=16,'Settings content must retain card insets');
  }else if(os==='android'){
   await page.getByRole('heading',{name:'关于',exact:true}).scrollIntoViewIfNeeded();await shot('general-lower');
  }
  assert.equal(await page.getByRole('heading',{name:'软件更新',exact:true}).count(),1);
  await shot('updates-idle');
  await page.locator('.prototype-state-controls>summary').click();await click('update:outcome:failed');
  await click('update:check');await page.locator('[data-update-page][aria-busy=false]').waitFor();
  await page.getByRole('alert').filter({hasText:'检查失败'}).waitFor();await shot('updates-failed');
  await page.locator('.prototype-state-controls>summary').click();await click('update:outcome:incompatible_data');await click('update:check');
  await page.getByRole('alert').filter({hasText:'数据格式不兼容'}).waitFor();await shot('updates-incompatible');
  assert.equal(await page.locator('[data-action="update:steps"]').count(),0);
  await page.locator('.prototype-state-controls>summary').click();await click('update:outcome:available');await click('update:check');
  await page.locator('[data-action="update:steps"]').waitFor();await shot('updates-available');
  await click('update:steps');await page.getByRole('dialog').waitFor();await shot('install-preview');await click('update:close');
  if(width<=700){
   await click('im:back:settings');
   await click('im:nav:activity');await click('im:nav:plans');await shot('activity-plans');await click('im:back:activity');
   await click('im:nav:information');await shot('information');
   for(const child of ['knowledge','memory','data']){await click('im:nav:'+child);await shot('information-'+child);await click('im:back:information');}
   await settings();await click('im:nav:models');
   await page.getByRole('button',{name:/五档推理 · 演示模型/}).click();await click('im:reasoning');
   const choices=page.locator('dialog [data-action^="im:effort:"]');assert.equal(await choices.count(),6);
   await shot('efforts');await click('im:effort:xhigh');await page.getByRole('button',{name:/推理档位 xhigh/}).waitFor();
   await click('im:model-default');await click('im:back:models');await click('im:back:settings');await click('im:nav:conversations');
   await page.getByRole('button',{name:/周末，去山里走走/}).click();await click('im:composer-effort');await click('im:effort:high');
   assert.equal(await page.locator('.im-effort-control').innerText(),'high');
   const editorBounds=await page.locator('#compose').boundingBox(), toolbarBounds=await page.locator('.im-composer-toolbar').boundingBox();
   assert.ok(editorBounds.width>=width-56 && toolbarBounds.y>=editorBounds.y+editorBounds.height-1,'writing area must use its own full-width row');
   await shot('chat-composer');await click('im:back:conversations');
   await settings();await click('im:nav:capabilities');
  }else{
   await settings();await click('ex-settings:agent');await click('ex-route:capabilities');
  }
  await page.locator('.prototype-state-controls>summary').click();await click('mg-catalog-fail');
  await page.getByRole('alert').filter({hasText:'无法读取能力清单'}).waitFor();await shot('capabilities-failed');
  await click('mg-catalog-reload');await click('mg-cap-tab:MCP');await click('mg-cap:mcp');await click('mg-configure:mcp');
  const field=page.getByLabel('名称',{exact:true});const original=await field.inputValue();await field.fill('保留未确认的配置草稿');
  await page.locator('dialog .prototype-state-controls>summary').click();await click('mg-save-unconfirmed');
  await page.getByRole('button',{name:'保存演示配置',exact:true}).click();
  await page.locator('.save-recovery:not([hidden])').waitFor();assert.equal(await field.inputValue(),'保留未确认的配置草稿');
  assert.equal(await page.getByRole('button',{name:'保存演示配置',exact:true}).isDisabled(),true);await shot('save-unconfirmed');
  await click('mg-reread-config');assert.ok((await page.locator('.save-recovery').innerText()).includes(original));assert.equal(await field.inputValue(),'保留未确认的配置草稿');
  await field.fill(original);await page.keyboard.press('Escape');await page.getByRole('dialog').waitFor({state:'hidden'});
  if(width===1440){
   await page.goto(base);
   const tab=await page.locator('.studio-center').getAttribute('data-tabkey');
   assert.equal(await page.locator('#work-tool').count(),0,'inspector has no page selector');
   for(const name of ['进度','目标','上下文'])await click('dk-section:'+name);
   for(const name of ['工作概览','进度','目标','上下文'])assert.equal(await page.locator(`[data-action="dk-section:${name}"]`).getAttribute('aria-expanded'),'true');
   await page.locator('.inspector-knowledge summary').filter({hasText:/^Wiki$/}).click();
   await page.getByLabel('Wiki草稿',{exact:true}).fill('保留未保存的 Wiki 演示草稿');
   await click('dk-section:上下文');await click('dk-section:上下文');
   assert.equal(await page.getByLabel('Wiki草稿',{exact:true}).inputValue(),'保留未保存的 Wiki 演示草稿');
   assert.equal(await page.locator('.studio-center').getAttribute('data-tabkey'),tab,'inspector never navigates the main content');
   await shot('inspector-expanded');
   for(const name of ['工作概览','进度','目标'])await click('dk-section:'+name);
   await page.getByLabel('Wiki草稿',{exact:true}).scrollIntoViewIfNeeded();await shot('inspector-context');
  }
  report.cases.push({width,scheme,os,passed:true});await context.close();
 }
 // Separate negative clipboard and edit-resend sequence; all messages remain synthetic.
 const context=await browser.newContext({viewport:{width:390,height:844},colorScheme:'dark',reducedMotion:'reduce'});
 const page=await context.newPage();page.setDefaultTimeout(8000);page.on('pageerror',e=>report.errors.push({flow:'message-actions',message:e.message}));
 await page.addInitScript(()=>{localStorage.setItem('nola-prototype-theme','dark');Object.defineProperty(navigator,'clipboard',{value:{writeText:()=>Promise.reject(new Error('fixture clipboard denied'))}});});
 await page.goto(base);await page.getByRole('button',{name:/周末，去山里走走/}).click();
 const composer=page.locator('#compose');await composer.fill('验证原型编辑的发送配置');await page.locator('[data-action="im:send"]').click();
 await page.locator('[data-action="im:stop"]').click();await composer.fill('未发送草稿应保留');
 await page.locator('[data-action="im:nav:im-details"]').click();await page.locator('[data-action="im:nav:models"]').click();
 await page.getByRole('button',{name:/Claude · 深入/}).click();await page.locator('[data-action="im:model-default"]').click();
 for(const action of ['im:back:models','im:back:im-details','im:back:chat'])await page.locator(`[data-action="${action}"]`).click();
 const edit=page.getByRole('button',{name:'编辑并重发',exact:true}).last();await edit.click();
 assert.ok((await page.getByRole('dialog').innerText()).includes('Qwen · 日常'),'edit retains send-time model after current model changes');
 await page.getByRole('button',{name:'取消',exact:true}).click();assert.equal(await composer.inputValue(),'未发送草稿应保留');
 await edit.click();await page.getByLabel('消息内容',{exact:true}).fill('修改后的独立分支');await page.getByRole('button',{name:'保存并重发送',exact:true}).click();
 await page.getByRole('button',{name:'查看原会话',exact:true}).waitFor();await page.screenshot({path:out+'/edited-branch.png'});
 await page.getByRole('button',{name:'查看原会话',exact:true}).click();assert.equal(await composer.inputValue(),'未发送草稿应保留');
 const copy=page.getByRole('button',{name:'复制消息',exact:true}).first();await copy.hover();assert.equal(await copy.evaluate(e=>getComputedStyle(e).backgroundColor),'rgba(0, 0, 0, 0)');
 await copy.click();await page.getByRole('heading',{name:'手动复制消息',exact:true}).waitFor();await page.screenshot({path:out+'/clipboard-denied.png'});
 report.cases.push({flow:'edit-cancel-branch-original-draft-frozen-model-and-clipboard-denied',passed:true});await context.close();
 assert.deepEqual(report.errors,[]);assert.deepEqual(report.externalRequests,[]);
 console.log(`PASS ${report.cases.length} flows, ${report.screens.length+2} captures; no external requests`);
 }finally{fs.writeFileSync(out+'/verification.json',JSON.stringify(report,null,2));await browser.close();}
})().catch(e=>{console.error(e);process.exitCode=1});
