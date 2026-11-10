// SPDX-License-Identifier: Apache-2.0
// A fresh browser/Host process must use persisted context, not replay the audit.
const {chromium}=require('playwright-core');
const fs=require('node:fs');
const path=require('node:path');
const assert=require('node:assert/strict');
const {query}=require('./real_model_evidence.cjs');
const out=process.env.NOLA_REMOTE_LONG_OUT, base=process.env.NOLA_REMOTE_LONG_URL;
const prior=JSON.parse(fs.readFileSync(path.join(out,'browser-receipt.json')));
const input=JSON.parse(fs.readFileSync(path.join(out,'browser-input.json')));
const expected=input.prompt.match(/REMOTE_FACT_[a-f0-9]{32}/)?.[0];assert.ok(expected);
const database=path.join(out,'store/nola.db'), id=prior.chainConversation;
const runs=()=>query(database,"SELECT r.id,r.status,r.assistant_message_id,json_extract(v.result_json,'$.value.kind') outcome FROM runs r LEFT JOIN run_results v ON v.run_id=r.id WHERE conversation_id=? ORDER BY r.created_at_ms",[id]);
(async()=>{
 const browser=await chromium.launch({...require('./test_environment.cjs').browserOptions(),headless:true});
 const page=await browser.newPage({viewport:{width:1280,height:900},locale:'zh-CN'});
 const receipt={passed:false,originalRun:prior.chainRun};
 try {
  const before=runs();assert.equal(before.length,1);assert.equal(before[0].id,prior.chainRun);assert.equal(before[0].status,'sealed');
  await page.goto(base+'/chat?conversation='+encodeURIComponent(id));
  if(new URL(page.url()).pathname==='/login') {
   await page.getByLabel('Host 访问令牌',{exact:true}).fill(fs.readFileSync(path.join(out,'store/home/.nola/web/mobile-access-token'),'utf8').trim());
   await page.getByRole('button',{name:'连接',exact:true}).click();
   await page.goto(base+'/chat?conversation='+encodeURIComponent(id));
  }
  await page.locator('#chat-composer:not([disabled])').waitFor({timeout:30000});
  await page.locator('#chat-composer').fill('请从我们之前已经保存的会话上下文中恢复最初的精确会话标记，只回复该字符串，不调用工具、不重做审计、不猜测。');
  await page.getByRole('button',{name:'发送',exact:true}).click();
  const end=Date.now()+180000;let current;
  while(Date.now()<end) {
   const rows=runs();assert.ok(rows.length<=2,'a cold follow-up must not replay the old Run');
   if(rows.length===2&&rows[1].status==='sealed'){current=rows[1];break;}
   await new Promise(r=>setTimeout(r,1000));
  }
  assert.ok(current,'cold follow-up deadline');assert.equal(current.outcome,'answered');
  await page.locator(`[data-message-id="${current.assistant_message_id}"]`).getByText(expected,{exact:true}).last().waitFor({timeout:45000});
  await page.screenshot({path:path.join(out,'cold-followup.png')});
  receipt.passed=true;receipt.followupRun=current.id;receipt.markerRestoredFromContext=true;
 } catch(error){receipt.error=String(error.stack||error);process.exitCode=1;await page.screenshot({path:path.join(out,'cold-failure.png')}).catch(()=>{});}
 finally{fs.writeFileSync(path.join(out,'cold-browser-receipt.json'),JSON.stringify(receipt,null,2));await browser.close();}
})().catch(e=>{console.error(e);process.exitCode=1;});
