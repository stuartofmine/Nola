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
if(!['failures','injection'].includes(process.env.REAL_DELEGATION_PHASE)){
const cancelled=await create();const files=[1,2].map(n=>`${root}/workspace/delegated-${cancelled}-${n}.txt`);
await send(`请真实并行委派两个独立助手。助手一实际每秒向 ${files[0]} 追加序号，持续90秒；助手二同样写 ${files[1]} 持续90秒。仅在两者完成后汇总，不要自行代替执行。`);
await until(()=>children(cancelled),v=>v.length===2&&v.every(x=>!x.ended_at_ms));await until(()=>files.map(f=>fs.existsSync(f)),v=>v.every(Boolean));
await p.locator('.composer-actions button:not([type=submit])').click();assert.equal((await terminal(cancelled)).outcome,'cancelled');
const stopped=await until(()=>children(cancelled),v=>v.length===2&&v.every(x=>x.ended_at_ms));await record('delegation-cancel-terminal',cancelled);assert.ok(stopped.every(x=>x.outcome==='cancelled'),'all child runs must cancel');const sizes=files.map(f=>fs.statSync(f).size);await p.waitForTimeout(2500);assert.deepEqual(files.map(f=>fs.statSync(f).size),sizes);await record('delegation-cancel',cancelled);
}
if(process.env.REAL_DELEGATION_PHASE!=='injection'){const toolFailure=await create();await send(`请真实委派一个助手读取不存在的文件 ${root}/workspace/missing-${toolFailure}.txt。不可创建或修复文件。助手须报告实际工具错误，你收到后明确说明读取失败原因，不要捏造内容。`);assert.equal((await terminal(toolFailure)).outcome,'answered');assert.equal(children(toolFailure).length,1);await record('delegation-tool-failure',toolFailure);}
const failed=await create(),childMarker='CHILD_PROVIDER_FAILURE_'+Date.now(),parentMarker='PARENT_FAILURE_SCENARIO_'+Date.now();fs.writeFileSync(fault,JSON.stringify({enabled:true,child_marker:childMarker,parent_marker:parentMarker}));
try{await send(`${parentMarker} 请真实委派一个助手，任务文本以 ${childMarker} 开头，计算31乘37。等待子任务终态；若失败请明确报告失败，不要重试或创建替代助手。`);await terminal(failed);const rows=children(failed);await record('delegation-provider-failure-observed',failed);assert.equal(rows.length,1);assert.equal(rows[0].outcome,'failed','injected provider error must propagate to child terminal');await record('delegation-provider-failure-injected',failed);}finally{fs.writeFileSync(fault,JSON.stringify({enabled:false}));}
console.log('PASS recorded delegation scenarios:', Object.keys(evidence).join(', '));
}finally{fs.writeFileSync(fault,JSON.stringify({enabled:false}));await b.close();}})().catch(e=>{console.error(e);process.exitCode=1});
