// Real production Host/browser/provider. No mocked responses or route interception.
const {chromium}=require('playwright-core');
const fs=require('node:fs');
const assert=require('node:assert/strict');
const {query}=require('./real_model_evidence.cjs');
const base=process.env.NOLA_LONG_WEB_URL, out=process.env.NOLA_LONG_WEB_OUT;
const db=out+'/store/nola.db';
let permissionPage;
const report={passed:false,pageErrors:[],scenarios:{}};
const runs=id=>query(db,"SELECT r.id,r.status,v.result_json FROM runs r LEFT JOIN run_results v ON v.run_id=r.id WHERE r.conversation_id=? ORDER BY r.created_at_ms",[id]);
async function until(f,timeout=240000){const start=Date.now();for(;;){const value=await f();if(value)return value;if(permissionPage){const panel=permissionPage.locator('.permission-panel').filter({has:permissionPage.getByRole('button',{name:'仅允许本次',exact:true})});if(await panel.count()){const button=panel.getByRole('button',{name:'仅允许本次',exact:true});if(await button.isEnabled()){assert.match(await panel.innerText(),/python3 long_job\.py/);await permissionPage.screenshot({path:out+'/web-permission-request.png'});await button.click();report.scenarios.onceOnlyPermission=true;}}}if(Date.now()-start>timeout)throw Error('Observation deadline');await new Promise(r=>setTimeout(r,350));}}
(async()=>{
 const browser=await chromium.launch({...require('./test_environment.cjs').browserOptions(),headless:true});
 const context=await browser.newContext({viewport:{width:1440,height:960},colorScheme:'dark',locale:'zh-CN'});
 let page=await context.newPage(); permissionPage=page;page.setDefaultTimeout(30000);
 const attach=p=>p.on('pageerror',e=>report.pageErrors.push(e.message));attach(page);
 try{
  await page.goto(base+'/chat');
  if(new URL(page.url()).pathname==='/login'){
   await page.getByLabel('Host 访问令牌',{exact:true}).fill(fs.readFileSync(out+'/store/home/.nola/web/mobile-access-token','utf8').trim());
   await page.getByRole('button',{name:'连接',exact:true}).click();
  }
  async function create(){
   const button=page.locator('.new-conversation');
   await page.waitForTimeout(400);
   if(!await button.isVisible())await page.locator('.navigation-toggle').click();
   const response=page.waitForResponse(r=>new URL(r.url()).pathname==='/api/chat/conversations'&&r.request().method()==='POST');
   await button.click();const body=await(await response).json();await page.waitForURL(u=>u.searchParams.get('conversation')===body.conversationId);
   await page.locator('#chat-composer:not([disabled])').waitFor();return body.conversationId;
  }
  async function send(text){await page.locator('#chat-composer').fill(text);await page.getByRole('button',{name:'发送',exact:true}).click();}
  const id=await create();report.owner=id;
  await send('请实际执行隔离项目中的 python3 long_job.py，等待结束后读取 result.json，核验 total=465、steps=30，并用 LONG_WEB_DONE 开头报告。不要改脚本、不要启动第二次，不要联网或派生助手。');
  await until(()=>fs.existsSync(out+'/store/project/progress.txt'));
  assert.equal(runs(id).at(-1).status,'running');
  await page.screenshot({path:out+'/web-long-running-dark.png'});
  await page.setViewportSize({width:390,height:844});
  const other=await create();assert.notEqual(other,id);
  await page.locator('#chat-composer').fill('UNSENT_OTHER_DRAFT');
  assert.equal(runs(id).at(-1).status,'running');
  report.scenarios.switchWhileRunning=true;
  await page.goto(base+'/chat?conversation='+encodeURIComponent(id));
  await page.locator('#chat-composer').waitFor();
  // Cold tab with normal persistent browser storage must observe the same Run.
  await page.close(); page=await context.newPage();permissionPage=page;page.setDefaultTimeout(30000);attach(page);
  await page.goto(base+'/chat?conversation='+encodeURIComponent(id));
  const terminal=await until(()=>{const r=runs(id);return r.at(-1)?.status==='sealed'?r.at(-1):null;});
  assert.ok(['answered','action_completed'].includes(JSON.parse(terminal.result_json).value.kind));
  await page.getByText(/LONG_WEB_DONE/).last().waitFor({timeout:45000});
  assert.deepEqual(JSON.parse(fs.readFileSync(out+'/store/project/result.json')),{total:465,steps:30});
  assert.equal(fs.readFileSync(out+'/store/project/starts.txt','utf8'),'start\n');
  report.scenarios.singleExecutionAndColdTab=true;
  await page.screenshot({path:out+'/web-long-completed-mobile.png'});
  await send('Use only the result we just verified. What was the total? Reply exactly WEB_RECALL_465. Do not use tools.');
  await until(()=>runs(id).length===2&&runs(id).at(-1).status==='sealed');
  await page.getByText('WEB_RECALL_465',{exact:true}).last().waitFor();
  report.scenarios.englishFollowup=true;
  await page.goto(base+'/chat?conversation='+encodeURIComponent(other));
  await until(async()=>await page.locator('#chat-composer').inputValue()==='UNSENT_OTHER_DRAFT');
  report.scenarios.otherDraftPreserved=true;
  assert.equal(report.pageErrors.length,0);
  fs.writeFileSync(out+'/web-accessibility.txt',await page.locator('body').innerText());
  await page.screenshot({path:out+'/web-draft-preserved-mobile.png'});
  report.passed=true;
 }catch(error){report.error=String(error.stack||error);await page.screenshot({path:out+'/web-failure.png'}).catch(()=>{});process.exitCode=1;}
 finally{fs.writeFileSync(out+'/web-receipt.json',JSON.stringify(report,null,2));await browser.close();}
})().catch(error=>{console.error(error);process.exitCode=1;});
