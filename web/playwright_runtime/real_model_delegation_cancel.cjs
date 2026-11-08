// Real parent/child execution. Provider failure scenario is explicitly fault-injected.
const {chromium}=require('playwright-core');
const assert=require('node:assert/strict');const fs=require('node:fs');const {query}=require('./real_model_evidence.cjs');
const root='/tmp/nola-web-real-20261020', artifacts='/tmp/nola-convergence-20261021/web', base='http://127.0.0.1:5159';
const db=root+'/state/nola.db', fault=artifacts+'/child-fault.json';
const sql=`SELECT r.id,r.conversation_id,r.created_at_ms,r.completed_at_ms,json_extract(v.result_json,'$.value.kind') outcome FROM runs r LEFT JOIN run_results v ON v.run_id=r.id WHERE r.conversation_id=? ORDER BY r.created_at_ms`;
const children=id=>query(db,`SELECT b.run_id,b.child_session_id,t.status,t.started_at_ms,t.ended_at_ms,json_extract(v.result_json,'$.value.kind') outcome FROM durable_tasks t JOIN runtime_task_run_bindings b ON b.task_id=t.id LEFT JOIN run_results v ON v.run_id=b.run_id WHERE t.owner_session_id=?`,[id]);
async function until(read,predicate,timeout=300000){const start=Date.now();for(;;){const value=read();if(predicate(value))return value;if(Date.now()-start>timeout)throw Error('Timed out: '+JSON.stringify(value));await new Promise(r=>setTimeout(r,500));}}
(async()=>{const b=await chromium.launch({executablePath:'/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',headless:true});const evidence={};try{const p=await b.newPage({viewport:{width:390,height:844}});await p.goto(base+'/chat');await p.locator('.new-conversation').waitFor({state:'attached'});
async function create(){if(!await p.locator('.new-conversation').isVisible())await p.locator('.navigation-toggle').click();const [r]=await Promise.all([p.waitForResponse(r=>r.url()===base+'/api/chat/conversations'&&r.request().method()==='POST'),p.locator('.new-conversation').click()]);const value=await r.json();const id=value.conversationId??value.conversation.conversationId;await p.waitForURL(u=>u.searchParams.get('conversation')===id);return id;}
async function send(text){await p.locator('#chat-composer:not([disabled])').fill(text);await p.locator('.composer-actions button[type=submit]').click();}
async function terminal(id){return (await until(()=>query(db,sql,[id]),v=>v.length&&v.at(-1).completed_at_ms)).at(-1);}
async function record(name,id){await p.waitForTimeout(800);evidence[name]={conversation:id,parent:query(db,sql,[id]),children:children(id),text:await p.locator('body').innerText()};await p.screenshot({path:artifacts+'/'+name+'.png'});fs.writeFileSync(artifacts+'/delegation-failure-results.json',JSON.stringify(evidence,null,2));}
const id=await create(),file=`${root}/workspace/cancel-fixed-${id}.txt`;
await send(`请真实委派一个独立助手，使用bash每秒向 ${file} 追加序号持续120秒。你等待助手最终完成后再汇报，不要自己代替执行。`);
await until(()=>children(id),v=>v.length===1&&!v[0].ended_at_ms,300000);
await until(()=>fs.existsSync(file)?fs.statSync(file).size:0,v=>v>0,300000);
const requestedAt=Date.now();const [response]=await Promise.all([p.waitForResponse(r=>r.url().endsWith('/api/chat/agent/v2/cancel')&&r.request().method()==='POST'),p.locator('.composer-actions button:not([type=submit])').click()]);
const receipt=await response.json();assert.equal(response.status(),202);console.log('cancel receipt',JSON.stringify(receipt));
fs.writeFileSync(artifacts+'/delegation-cancel-last-request.json',JSON.stringify({conversation:id,requestedAt,httpStatus:response.status(),receipt},null,2));
const parent=await terminal(id);assert.equal(parent.outcome,'cancelled');
const child=await until(()=>children(id),v=>v.length===1&&v[0].ended_at_ms,120000);assert.equal(child[0].outcome,'cancelled');
const stoppedSize=fs.statSync(file).size;await p.waitForTimeout(3000);assert.equal(fs.statSync(file).size,stoppedSize,'child physical writes must remain stopped');
evidence.cancellation={conversation:id,requestedAt,httpStatus:response.status(),receipt,parent,children:child,stoppedSize,stableForMs:3000};
await p.locator('.composer-actions button[type=submit]').waitFor();
await p.locator('.reply-error[data-kind=uncertain]').waitFor({state:'hidden'});
await p.screenshot({path:artifacts+'/delegation-cancel-fixed.png'});
await send('上一次委派已经取消。请仅回复：父子取消后恢复正常。');const recovery=await until(()=>query(db,sql,[id]),v=>v.length===2&&v[1].completed_at_ms,300000);assert.equal(recovery[1].outcome,'answered');evidence.recovery=recovery[1];
await p.locator('.composer-actions button[type=submit]').waitFor();
await p.locator('.reply-error[data-kind=uncertain]').waitFor({state:'hidden'});
evidence.uiSettledAt=Date.now();
await p.screenshot({path:artifacts+'/delegation-cancel-recovery.png'});fs.writeFileSync(artifacts+'/delegation-cancel-fixed.json',JSON.stringify(evidence,null,2));console.log('PASS real parent child cancellation, physical stop and recovery');
}finally{await b.close();}})().catch(e=>{console.error(e);process.exitCode=1});
