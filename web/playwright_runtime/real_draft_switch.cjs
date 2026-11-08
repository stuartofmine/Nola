// Real Chrome and production Host; no mocked API or provider calls.
const {chromium}=require('playwright-core');
const fs=require('node:fs');
const assert=require('node:assert/strict');
(async()=>{
 const state=JSON.parse(fs.readFileSync(process.env.NOLA_BUTTON_AUDIT_HOST_STATE));
 const out=process.env.NOLA_DRAFT_AUDIT_OUT||'/tmp/nola-real-draft-switch';fs.mkdirSync(out,{recursive:true});
 const browser=await chromium.launch({executablePath:'/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',headless:false});
 const page=await browser.newPage({viewport:{width:1400,height:1000}});page.setDefaultTimeout(30000);
 const report={errors:[],created:[]};page.on('pageerror',e=>report.errors.push(e.message));
 try {
  await page.goto(state.url+'/login');
  await page.getByLabel('Host 访问令牌',{exact:true}).fill(fs.readFileSync(state.root+'/home/.nola/web/mobile-access-token','utf8').trim());
  await page.getByRole('button',{name:'连接',exact:true}).click();
  const create=page.getByRole('button',{name:'新建会话',exact:true});await create.waitFor();
  async function newConversation(){const res=page.waitForResponse(r=>new URL(r.url()).pathname==='/api/chat/conversations'&&r.request().method()==='POST');await create.click();const body=await(await res).json();report.created.push(body.conversationId);await page.waitForURL(u=>u.searchParams.get('conversation')===body.conversationId);return body.conversationId;}
  const first=await newConversation();const editor=page.locator('#chat-composer');
  await editor.fill('DRAFT_FIRST_PRESERVED');
  const box=await editor.boundingBox();
  await page.evaluate(()=>{window.draftSwitchSamples=[];const e=document.querySelector('#chat-composer');new MutationObserver(()=>window.draftSwitchSamples.push({disabled:e.disabled,busy:e.getAttribute('aria-busy'),url:location.href})).observe(e,{attributes:true,attributeFilter:['disabled','aria-busy']});});
  const response=page.waitForResponse(r=>new URL(r.url()).pathname==='/api/chat/conversations'&&r.request().method()==='POST');
  await create.click();
  report.immediatelyDisabled=await editor.isDisabled();assert.equal(report.immediatelyDisabled,true,'old composer must lock immediately after Create');
  await page.mouse.click(box.x+20,box.y+20);await page.keyboard.type('RACE_TEXT');
  const second=(await(await response).json()).conversationId;report.created.push(second);
  await page.waitForURL(u=>u.searchParams.get('conversation')===second);
  await page.waitForFunction(()=>!document.querySelector('#chat-composer').disabled);
  assert.equal(await editor.inputValue(),'','transition keystrokes must not enter either conversation');
  await editor.fill('DRAFT_SECOND_PRESERVED');
  await page.reload();await page.waitForFunction(()=>document.querySelector('#chat-composer')&&!document.querySelector('#chat-composer').disabled);
  assert.equal(await editor.inputValue(),'DRAFT_SECOND_PRESERVED');
  await page.goto(state.url+'/chat?conversation='+encodeURIComponent(first));
  await page.waitForFunction(()=>document.querySelector('#chat-composer')&&!document.querySelector('#chat-composer').disabled);
  assert.equal(await editor.inputValue(),'DRAFT_FIRST_PRESERVED');
  await page.waitForFunction(()=>!document.querySelector('button.primary[type=submit]').disabled);report.sendEnabled=true;report.restoredBoth=true;await page.screenshot({path:out+'/draft-preserved.png'});
 }catch(e){report.failure=String(e.stack||e);await page.screenshot({path:out+'/failure.png'}).catch(()=>{});process.exitCode=1;}
 finally{fs.writeFileSync(out+'/report.json',JSON.stringify(report,null,2));await browser.close();}
})();
