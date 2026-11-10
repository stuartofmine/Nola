// Real model/Host flow, one browser page retains the originating session identity.
const {chromium}=require('playwright-core');const fs=require('node:fs');const assert=require('node:assert/strict');
(async()=>{
 const state=JSON.parse(fs.readFileSync(process.env.NOLA_A2UI_HOST_STATE||require('./test_environment.cjs').artifactPath('nola-a2ui-real-host-state.json'),'utf8'));
 const cid=process.env.NOLA_A2UI_TEST_CONVERSATION;assert.ok(cid);
 const origin=new URL(state.url);assert.equal(origin.hostname,'127.0.0.1');assert.ok(['5141','5142'].includes(origin.port));
 const out=process.env.NOLA_A2UI_EVIDENCE_DIR||require('./test_environment.cjs').artifactPath('nola-a2ui-real-flow');assert.ok(require('node:path').basename(out).startsWith('nola-a2ui-'));fs.mkdirSync(out,{recursive:true});
 const browser=await chromium.launch({...require('./test_environment.cjs').browserOptions(),headless:true});
 const context=await browser.newContext({viewport:{width:1280,height:900}});const page=await context.newPage();
 const followupOnly=process.env.NOLA_A2UI_RESUME_FOLLOWUP==='1';
 const resume=process.env.NOLA_A2UI_RESUME==='1'||followupOnly;
 const terminalTimeout=Number(process.env.NOLA_A2UI_TERMINAL_TIMEOUT_MS||180000);
 assert.ok(Number.isSafeInteger(terminalTimeout)&&terminalTimeout>=180000&&terminalTimeout<=600000);
 if(resume)await page.addInitScript(id=>sessionStorage.setItem('nola.web.chat.client',id),JSON.parse(fs.readFileSync(out+'/client.json')).client_id);
 else if(process.env.NOLA_A2UI_CLIENT_ID)await page.addInitScript(id=>sessionStorage.setItem('nola.web.chat.client',id),process.env.NOLA_A2UI_CLIENT_ID);
 const errors=[];page.on('pageerror',e=>errors.push(e.name));
 const network=[];
 page.on('request',request=>{
  const url=new URL(request.url());
  if(url.pathname.endsWith('/ui-interactions')||url.pathname.endsWith('/ui-surface-preview')||url.pathname.endsWith('/stream')) {
   network.push({event:'request',path:url.pathname,method:request.method(),at:Date.now()});
   fs.writeFileSync(`${out}/network-status.json`,JSON.stringify(network,null,2),{mode:0o600});
  }
 });
 page.on('response',response=>{
  const url=new URL(response.url());
  if(url.pathname.startsWith('/api/')&&(response.status()>=400||url.pathname.endsWith('/queued-input')||url.pathname.endsWith('/stream')||url.pathname.endsWith('/ui-interactions'))){
   network.push({event:'response',path:url.pathname,status:response.status(),at:Date.now()});
   fs.writeFileSync(`${out}/network-status.json`,JSON.stringify(network,null,2),{mode:0o600});
  }
 });
 const save=(name,data)=>fs.writeFileSync(`${out}/${name}.json`,JSON.stringify(data,null,2),{mode:0o600});
 save('result',{kind:'real-model-same-context',conversation:cid,status:'running',passed:[],started_at:new Date().toISOString()});
 const read=path=>page.evaluate(async path=>(await fetch(path)).json(),`/api/chat/conversations/${encodeURIComponent(cid)}/${path}`);
 async function assertActionSummary(projection,label) {
  const message=projection.messages.findLast(m=>m.role==='user'&&m.parts.some(p=>p.kind==='ui_action'));
  assert.ok(message,'real interaction must persist a typed ui_action user Part');
  const part=message.parts.find(p=>p.kind==='ui_action');
  assert.ok(part.summary?.trim());assert.equal(part.envelope.type,'ui_action');
  assert.equal(part.surface.identity.owner.conversation_id,cid);
  assert.notEqual(part.surface.identity.owner.message_id,message.message_id,'action retains original Surface owner');
  assert.ok(JSON.stringify(part.envelope).includes('同一浏览器连续验证'),'model envelope retains submitted input');
  const exactRow=page.locator(`article[data-message-id=${JSON.stringify(message.message_id)}]`);
  await exactRow.waitFor({timeout:30000});
  const displayed=await exactRow.innerText();
  assert.ok(displayed.includes(part.summary),'user bubble shows Core summary');
  assert.ok(!displayed.includes('"ui_action"')&&!displayed.includes('"originating_run_id"'),'user bubble must not expose raw action JSON');
  save(label,{message_id:message.message_id,summary:part.summary,typed:true,summary_visible:true});
 }
 async function terminal(minimum,label) {
  const deadline=Date.now()+terminalTimeout;let approvals=0;let resumed=false;
  while(Date.now()<deadline){
   const recovery=page.locator('.recovery');
   if(!resumed&&await recovery.isVisible()) {
    const resumeButton=recovery.getByRole('button',{name:'恢复当前运行',exact:true});
    if(await resumeButton.count()) { await resumeButton.click(); resumed=true; }
    else {
     const inspect=recovery.getByRole('button',{name:'检查记录',exact:true});
     if(await inspect.isEnabled())await inspect.click();
     // Unknown side-effect retry requires a separate explicit review; never auto-authorize it.
     if(await recovery.getByRole('button',{name:'授权重试',exact:true}).count())throw Error('unknown effect requires review');
    }
   }
   const panel=page.locator('.permission-panel');
   if(await panel.isVisible()){
    const tool=(await panel.locator('.tool').innerText()).trim();
    if(tool!=='render_ui')throw Error('unexpected permission '+tool);
    const allow=panel.locator('.actions button.primary');
    if(await allow.isEnabled()){await panel.screenshot({path:`${out}/${label}-approval.png`});await allow.click();approvals++;}
   }
   const messages=await read('messages');save(label+'-messages',messages);save(label+'-surfaces',await read('ui-surfaces'));
   if(messages.messages.length>=minimum&&!messages.conversationProjection.active_run&&messages.conversationProjection.messages.at(-1)?.state==='completed'){save(label+'-status',{terminal:true,approvals});return messages;}
   await page.waitForTimeout(1000);
  }
  throw Error(label+' did not reach terminal within '+terminalTimeout/1000+' seconds');
 }
 try {
  await page.goto(state.url,{waitUntil:'domcontentloaded'});
  await page.goto(`${origin.origin}/chat?conversation=${encodeURIComponent(cid)}`,{waitUntil:'domcontentloaded'});
  const surface=page.locator('.surface-reference').first();if(!followupOnly)await surface.waitFor({timeout:60000});
  save('client',{client_id:await page.evaluate(()=>sessionStorage.getItem('nola.web.chat.client'))});
  const before=followupOnly?null:resume?JSON.parse(fs.readFileSync(out+'/before-messages.json')):await read('messages');
  if(!resume)save('before-messages',before);
  const marker='同一浏览器连续验证';
  if(!resume){assert.equal(before.conversationProjection.active_run,null);save('before',await read('ui-surfaces'));
  const committedBeforePreview=await read('ui-surfaces');
  const interactionCount=()=>network.filter(e=>e.event==='request'&&e.path.endsWith('/ui-interactions')).length;
  const interactionsBefore=interactionCount();
  const previewResponsePromise=page.waitForResponse(r=>new URL(r.url()).pathname.endsWith('/ui-surface-preview')&&r.request().method()==='POST');
  await surface.locator('input:not([disabled]),textarea:not([disabled])').first().fill(marker);
  const previewResponse=await previewResponsePromise;
  assert.equal(previewResponse.status(),200,'real Core draft preview must succeed');
  const previewInput=previewResponse.request().postDataJSON();const preview=(await previewResponse.json()).preview;
  save('draft-preview',{input:previewInput,preview});
  const committed=committedBeforePreview.surfaces.find(s=>JSON.stringify(s.identity)===JSON.stringify(previewInput.identity));
  assert.ok(committed,'preview must belong to an existing full Surface identity');
  assert.equal(previewInput.identity.owner.conversation_id,cid);
  assert.equal(previewInput.expected_surface_revision,committed.revision);
  assert.ok(Number.isSafeInteger(previewInput.draft_generation)&&previewInput.draft_generation>0);
  assert.equal(Object.hasOwn(previewInput,'action'),false);
  assert.deepEqual(preview.identity,committed.identity);assert.deepEqual(preview.snapshot.identity,committed.identity);
  assert.equal(preview.base_revision,committed.revision);assert.equal(preview.snapshot.revision,committed.revision);
  assert.equal(preview.draft_generation,previewInput.draft_generation);
  assert.deepEqual(await read('ui-surfaces'),committedBeforePreview,'readonly preview must not mutate persisted surfaces');
  assert.equal(interactionCount(),interactionsBefore,'editing/preview must not submit an interaction');
  const responsePromise=page.waitForResponse(r=>new URL(r.url()).pathname.endsWith('/ui-interactions'));
  await surface.locator('button.a2ui-button:not([disabled])').first().click();const response=await responsePromise;const receipt=await response.json();
  save('interaction-request',response.request().postDataJSON());save('interaction-receipt',receipt);
  assert.equal(receipt.status,'committed');assert.ok(receipt.receipt.queued_submission_id);
  const submitted=response.request().postDataJSON().input;
  assert.deepEqual(submitted.interaction.identity,previewInput.identity);
  assert.equal(submitted.interaction.expected_surface_revision,previewInput.expected_surface_revision);
  assert.deepEqual(submitted.interaction.edits,previewInput.edits);
  assert.ok(submitted.interaction.action,'Core must atomically revalidate edits together with the action');
  assert.deepEqual(receipt.receipt.snapshot.identity,previewInput.identity);
  assert.ok(receipt.receipt.snapshot.revision>preview.base_revision);
  }
  const action=followupOnly?JSON.parse(fs.readFileSync(out+'/action-messages.json')):await terminal(before.messages.length+2,'action');assert.ok(JSON.stringify(action.messages.at(-1)).includes(marker));
  if(!followupOnly){
   await assertActionSummary(action.conversationProjection,'action-summary-before-reload');
   await page.reload({waitUntil:'domcontentloaded'});await surface.waitFor({timeout:60000});
   const reopened=await read('messages');await assertActionSummary(reopened.conversationProjection,'action-summary-after-reload');
  }
  assert.equal(await page.evaluate(()=>sessionStorage.getItem('nola.web.chat.client')),JSON.parse(fs.readFileSync(out+'/client.json')).client_id);
  if(!followupOnly){
  save('before-followup',await read('ui-surfaces'));
  await page.locator('#chat-composer').fill('请在这张表单上显示已提交的名称，保留输入和提交按钮');
  await page.locator('.composer-actions button[type=submit]').click();
  }
  await terminal(action.messages.length+2,'followup');
  const final=await read('ui-surfaces');save('final',final);const original=JSON.parse(fs.readFileSync(out+'/before-followup.json')).surfaces[0];
  const same=final.surfaces.find(s=>JSON.stringify(s.identity)===JSON.stringify(original.identity));assert.ok(same&&same.revision>original.revision,'followup must update original full Surface identity');
  assert.ok(JSON.stringify(same.nodes).includes(marker),'updated card must contain submitted name');
  const isolatedConversation=process.env.NOLA_A2UI_ISOLATION_CONVERSATION;
  if(isolatedConversation){
   assert.notEqual(isolatedConversation,cid);
   const isolated=await page.evaluate(async id=>(await fetch(`/api/chat/conversations/${encodeURIComponent(id)}/ui-surfaces`)).json(),isolatedConversation);
   save('isolation-after',isolated);
   assert.equal(isolated.surfaces.length,1);assert.equal(isolated.surfaces[0].revision,1);
   assert.equal(isolated.surfaces[0].identity.owner.conversation_id,isolatedConversation);
   assert.ok(!JSON.stringify(isolated.surfaces[0]).includes(marker),'submitted data must not cross conversations');
  }
  await page.setViewportSize({width:390,height:844});await surface.evaluate(e=>e.scrollIntoView({block:'start'}));await page.screenshot({path:out+'/mobile.png'});
  save('result',{kind:'real-model-same-context',conversation:cid,passed:['submit','action-terminal-with-value','same-client-reload','natural-followup-terminal','same-surface-update','mobile-render'],errors});assert.deepEqual(errors,[]);
  console.log('PASS real model same-context A2UI flow');
 }catch(error){save('result',{kind:'real-model-same-context',conversation:cid,passed:[],failed:true,error:error.name,errors});await page.screenshot({path:out+'/failure.png'}).catch(()=>{});console.error('Real flow failure',error.name,error.message.replace(/https?:\/\/[^\s]+/g,'[URL]'));process.exitCode=1;}
 finally{await browser.close();}
})().catch(e=>{console.error('Real flow initialization failure',e.name);process.exitCode=1});
