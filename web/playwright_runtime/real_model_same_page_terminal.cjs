// Real parent/child execution. Provider failure scenario is explicitly fault-injected.
const {chromium}=require('playwright-core');
const assert=require('node:assert/strict');const fs=require('node:fs');const {query}=require('./real_model_evidence.cjs');
const root='/tmp/nola-web-real-20261020', artifacts='/tmp/nola-convergence-20261021/web', base='http://127.0.0.1:5159';
const db=root+'/state/nola.db', fault=artifacts+'/child-fault.json';
const sql=`SELECT r.id,r.conversation_id,r.created_at_ms,r.completed_at_ms,json_extract(v.result_json,'$.value.kind') outcome FROM runs r LEFT JOIN run_results v ON v.run_id=r.id WHERE r.conversation_id=? ORDER BY r.created_at_ms`;
const children=id=>query(db,`SELECT b.run_id,b.child_session_id,t.status,t.started_at_ms,t.ended_at_ms,json_extract(v.result_json,'$.value.kind') outcome FROM durable_tasks t JOIN runtime_task_run_bindings b ON b.task_id=t.id LEFT JOIN run_results v ON v.run_id=b.run_id WHERE t.owner_session_id=?`,[id]);
async function until(read,predicate,timeout=300000){const start=Date.now();for(;;){const value=read();if(predicate(value))return value;if(Date.now()-start>timeout)throw Error('Timed out: '+JSON.stringify(value));await new Promise(r=>setTimeout(r,500));}}
(async()=>{const b=await chromium.launch({executablePath:'/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',headless:true});try{
const id=process.env.REAL_WEB_CONVERSATION;assert.ok(id,'explicit isolated conversation required');
const rows=query(db,"select identity_json from source_submission_receipts where json_extract(identity_json,'$.conversation_id')=? order by rowid desc limit 1",[id]);assert.equal(rows.length,1);
const p=await b.newPage({viewport:{width:390,height:844}});await p.addInitScript(client=>sessionStorage.setItem('nola.web.chat.client',client),JSON.parse(rows[0].identity_json).client_id);
const diagnostics=[];p.on('pageerror',e=>diagnostics.push({kind:'pageerror',message:e.message}));p.on('response',r=>{if(!r.ok())diagnostics.push({kind:'http',status:r.status(),path:new URL(r.url()).pathname});});
await p.goto(base+'/chat?conversation='+id);await p.locator('.composer-actions button[type=submit]').waitFor();await p.locator('.reply-error[data-kind=uncertain]').waitFor({state:'hidden'});
const before=query(db,sql,[id]).length;const startedAt=Date.now();await p.locator('#chat-composer:not([disabled])').fill('请仅回复：同页终态确认。');await p.locator('.composer-actions button[type=submit]').click();
const records=await until(()=>query(db,sql,[id]),v=>v.length===before+1&&v.at(-1).completed_at_ms,180000);assert.equal(records.at(-1).outcome,'answered');const canonicalObservedAt=Date.now();
await p.locator('.composer-actions button[type=submit]').waitFor({timeout:45000});await p.locator('.reply-error[data-kind=uncertain]').waitFor({state:'hidden',timeout:45000});await p.locator('#chat-composer:not([disabled])').waitFor();
assert.equal(await p.locator('.run-status').count(),0);const uiSettledAt=Date.now();await p.screenshot({path:artifacts+'/same-page-terminal.png'});
fs.writeFileSync(artifacts+'/same-page-terminal.json',JSON.stringify({conversation:id,startedAt,canonicalObservedAt,uiSettledAt,run:records.at(-1),samePage:true,diagnostics},null,2));console.log('PASS same-page terminal verification and Send restored');
}finally{await b.close()}})().catch(e=>{console.error(e);process.exitCode=1});
