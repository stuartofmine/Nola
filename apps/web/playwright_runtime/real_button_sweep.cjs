// Production UI and isolated Rust Host only; no intercepted API responses.
const {chromium}=require('playwright-core'),fs=require('node:fs'),assert=require('node:assert/strict');
(async()=>{
 const s=JSON.parse(fs.readFileSync(process.env.NOLA_BUTTON_AUDIT_HOST_STATE||(process.env.NOLA_A2UI_HOST_STATE || require('./test_environment.cjs').artifactPath('nola-a2ui-real-host-state.json'))));assert.ok(s.root.includes('nola-a2ui-real-'));
 const out=process.env.NOLA_BUTTON_AUDIT_OUT||require('./test_environment.cjs').artifactPath('nola-real-button-sweep');fs.mkdirSync(out,{recursive:true});
 const b=await chromium.launch({...require('./test_environment.cjs').browserOptions(),headless:process.env.NOLA_UI_HEADLESS==='1'});
 const p=await b.newPage({viewport:{width:1280,height:900},locale:'zh-CN'});p.setDefaultTimeout(20000);
 const report={clicked:[],screens:[],errors:[],failedRequests:[]};p.on('pageerror',e=>report.errors.push(e.message));
 const pendingRequests=new Map();
 p.on('request',r=>pendingRequests.set(r,{path:new URL(r.url()).pathname,method:r.method(),type:r.resourceType(),started:Date.now()}));
 p.on('requestfinished',r=>pendingRequests.delete(r));
 p.on('requestfailed',r=>pendingRequests.delete(r));
 p.on('requestfailed',r=>report.failedRequests.push({path:new URL(r.url()).pathname,error:r.failure()?.errorText}));
 p.on('response',r=>{if(r.status()>=400)report.failedRequests.push({path:new URL(r.url()).pathname,status:r.status()});});
 async function click(role,name){await p.getByRole(role,{name,exact:true}).click();report.clicked.push({route:new URL(p.url()).pathname,role,name});}
 async function capture(name){await p.screenshot({path:out+'/'+name+'.png',timeout:60000,animations:'disabled'});fs.writeFileSync(out+'/'+name+'.txt',await p.locator('body').ariaSnapshot());report.screens.push(name);}
 try{
  await p.goto(s.url+'/login');await p.getByLabel('Host 访问令牌',{exact:true}).fill(fs.readFileSync(s.root+'/home/.nola/web/mobile-access-token','utf8').trim());await click('button','连接');await p.waitForURL(u=>u.pathname!='/login');
  let owner;
  if(!process.env.NOLA_BUTTON_AUDIT_SECTIONS_ONLY){
  await p.goto(s.url+'/chat');const [created]=await Promise.all([p.waitForResponse(r=>new URL(r.url()).pathname==='/api/chat/conversations'&&r.request().method()==='POST'),click('button','新对话')]);const identity=await created.json();await p.waitForURL(u=>u.searchParams.get('conversation')===identity.conversationId);await p.locator('#chat-composer:not([disabled])').waitFor();owner=p.url();
  const csrf=(await p.context().cookies(s.url)).find(cookie=>cookie.name==='nola_web_csrf');
  assert.ok(csrf);
  const readExchanges=expectedEpoch=>p.request.post(s.url+'/api/agent-rooms',{
    headers:{'X-Nola-CSRF':csrf.value,Origin:s.url},
    data:{command:{type:'peer_exchanges',conversation_id:identity.conversationId,limit:40,offset:0},expected_source_store_epoch:expectedEpoch}
  });
  const peerPage=await readExchanges();
  assert.equal(peerPage.status(),200);
  const scoped=await peerPage.json();
  assert.equal(scoped.result.conversation_id,identity.conversationId);
  assert.deepEqual(scoped.result.exchanges,[]);
  assert.ok(scoped.source_matches && scoped.source_store_epoch);
  assert.equal((await readExchanges('different-store')).status(),409);
  report.expertHistoryScopeVerified=true;
  await p.locator('#chat-composer').fill('Reply exactly WEB_BUTTON_AUDIT_OK.');await click('button','发送');
  await p.locator('.transcript').getByText('WEB_BUTTON_AUDIT_OK',{exact:true}).waitFor({timeout:180000});await p.getByRole('button',{name:'发送',exact:true}).waitFor();await capture('real-reply');
  const replacement = 'Now reply exactly WEB_BUTTON_AUDIT_OK.';
  await click('button','编辑并重发');
  await p.getByRole('dialog').getByRole('textbox').fill(replacement);
  await click('button','保存并重发');
  const log=p.getByRole('log',{name:'对话'});
  await log.getByText(replacement,{exact:true}).waitFor();
  await log.getByText('WEB_BUTTON_AUDIT_OK',{exact:true}).waitFor({timeout:180000});
  await p.getByRole('button',{name:'编辑并重发',exact:true}).waitFor();
  assert.equal(p.url(),owner);
  await p.reload();
  await log.getByText(replacement,{exact:true}).waitFor();
  await log.getByText('WEB_BUTTON_AUDIT_OK',{exact:true}).waitFor();
  await p.getByRole('main',{name:replacement,exact:true}).waitFor();
  assert.equal(await log.getByRole('article').count(),2);
  assert.equal(await p.getByText('正在加载会话…',{exact:true}).isVisible(),false);
  await capture('edited-and-reloaded');
  await click('button','查找会话内容');await p.locator('#chat-search-input').fill('WEB_BUTTON');await p.locator('.chat-search button').click();report.clicked.push({name:'close search'});await click('button','更多工具');await click('button','刷新会话');
  for (const width of [1280,390]) {
   await p.setViewportSize({width,height:900});
   for (const name of ['任务','产物','上下文']) {
    if (width < 600 && name !== '上下文') {
     await click('button','更多工具');
     const menu=p.getByRole('dialog',{name:'更多工具'});
     assert.equal(await menu.getByRole('button',{name:'查找会话内容',exact:true}).count(),1);
     assert.equal(await menu.getByRole('button',{name:'上下文',exact:true}).count(),0);
     assert.equal(await menu.getByRole('button',{name:'专家',exact:true}).count(),0);
    }
    await click('button',name);
    const inspector=p.locator('.chat-side:visible');await inspector.waitFor();
    const sectionsByPanel={'任务':['overview','progress','goal'],'产物':['review','artifacts'],'上下文':['resources','execution','participants','settings']};
    assert.deepEqual(await inspector.locator('[data-inspector-section]:visible').evaluateAll(nodes=>nodes.map(node=>node.dataset.inspectorSection)),sectionsByPanel[name]);
    await capture(`inspector-${width}-${name}`);
    if (width < 600) {
     const dialog=p.locator('dialog:modal');await p.keyboard.press('Escape');await dialog.waitFor({state:'hidden'});
     report.clicked.push({name:'Escape dismiss inspector',width});
    } else await click('button',name);
   }
  }
  }
  await p.setViewportSize({width:1280,height:900});await p.goto(s.url+'/settings?section=general');for(const name of ['通用','模型服务','助手','连接','数据','诊断']){await click('tab',name);await capture('settings-'+name);const reload=p.getByRole('button',{name:'重新载入',exact:true});for(let i=0;i<await reload.count();i++){await reload.nth(i).click();report.clicked.push({name:'重新载入',section:name});}}await click('button','复制精简报告');
  await p.goto(s.url+'/data-sources');for(const name of ['资料','记忆','计划','记录']){await click('tab',name);const refresh=p.getByRole('main').getByRole('button',{name:'刷新',exact:true});for(let i=0;i<await refresh.count();i++){await refresh.nth(i).click();report.clicked.push({name:'刷新',section:name,index:i});}await capture('workspace-'+name);}await click('tab','资料');await click('button','添加数据源');await capture('new-source');await p.keyboard.press('Escape');
  await p.goto(s.url+'/capabilities');for(const [tab,button] of [['技能','新增技能'],['MCP','新增 MCP']]){await click('tab',tab);await click('button',button);await capture('new-'+tab);const cancel=p.getByRole('button',{name:'取消',exact:true});if(await cancel.isVisible())await click('button','取消');else await p.keyboard.press('Escape');}
  await click('tab','技能');await click('button','新增技能');await p.getByRole('dialog').getByLabel('名称',{exact:true}).fill('Unsaved audit draft');await click('button','取消');await p.getByRole('dialog',{name:'放弃修改？',exact:true}).waitFor();await click('button','继续编辑');assert.equal(await p.getByRole('dialog').getByLabel('名称',{exact:true}).inputValue(),'Unsaved audit draft');await click('button','取消');await click('button','放弃修改');await p.locator('dialog:modal').waitFor({state:'hidden'});
  if(owner){await p.goto(owner);await p.locator('.transcript').getByText('WEB_BUTTON_AUDIT_OK',{exact:true}).waitFor();await capture('restored');}assert.deepEqual(report.errors,[]);assert.deepEqual(report.failedRequests.filter(request=>request.status>=500),[],'Unexpected Host server error during the button sweep');report.passed=true;
 }catch(e){report.failure=e.message;report.pendingRequests=[...pendingRequests.values()].map(({started,...request})=>({...request,elapsedMs:Date.now()-started}));await capture('failure').catch(()=>{});process.exitCode=1;}finally{fs.writeFileSync(out+'/report.json',JSON.stringify(report,null,2));console.log(JSON.stringify({passed:report.passed,clicks:report.clicked.length,failure:report.failure,out}));await b.close();}
})();
