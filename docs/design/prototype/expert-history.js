// SPDX-License-Identifier: Apache-2.0
// Synthetic histories and explicit cooperation receipts; never reads a Host or invokes a model.
'use strict';
window.expertHistoryModule=(()=>{
 const expanded=new Set(), examples=new Map();let opened=null,initialParents=null;
 const contexts=parent=>window.expertRolesModule?.childContexts(parent)||[];
 function seed(parent){
  if(!initialParents)initialParents=new Set(conversations.map(c=>c.id));
  if(!initialParents.has(parent)||examples.has(parent))return;
  const members=contexts(parent).filter(c=>c.isCurrent);
  if(!members.length)return;
  examples.set(parent,members.map(target=>({exchangeID:'demo-exchange:'+target.contextID,taskID:'demo-exchange:'+target.contextID,parentID:parent,
   source:{kind:'main_agent',contextID:parent,name:'Nola'},
   target:{contextID:target.contextID,name:target.displayName,revision:target.revision},
   state:'completed',completionConfirmed:true,text:'请核对这份摘要中的论据与引用。',result:'已完成核对。引用与结论对应，有一项推断需要在正文中注明。',clarifications:[]})));
 }
 function records(parent,context){seed(parent);return (examples.get(parent)||[]).filter(e=>e.source.contextID===context||e.target.contextID===context);}
 function rows(parent){seed(parent);return `<div class="eh-tree"><button type="button" class="eh-toggle" data-action="eh:toggle:${encodeURIComponent(parent)}" aria-expanded="${expanded.has(parent)}"><span>专家会话</span><span aria-hidden="true">${expanded.has(parent)?'−':'+'}</span></button>${expanded.has(parent)?`<div class="eh-children">${contexts(parent).map(c=>`<button type="button" data-action="eh:open:${encodeURIComponent(parent)}:${encodeURIComponent(c.contextID)}"><span>${esc(c.displayName)}</span><small>${c.isCurrent?'当前上下文':'历史上下文 · 只读'}</small></button>`).join('')||'<small>还没有专家会话</small>'}</div>`:''}</div>`;}
 function exchangeRows(exchanges){
  if(!exchanges.length)return '';
  const status={queued:'等待开始',running:'正在协作',waiting_for_input:'等待澄清',cancel_requested:'正在停止',cancelled:'已停止',failed:'需要处理',completed:'已完成'};
  return `<section class="eh-exchanges" aria-label="专家任务">${exchanges.map(e=>`<details data-exchange-id="${esc(e.exchangeID)}" data-task-id="${esc(e.taskID)}"><summary><span>${esc(e.source.name)} <span aria-hidden="true">→</span> ${esc(e.target.name)}</span><small>${e.completionConfirmed?status[e.state]:(status[e.state]||'等待确认')}</small></summary><div class="eh-exchange-body"><small>请求</small><p>${esc(e.text)}</p>${e.clarifications.map(q=>`<small>${esc(q.author)} · 澄清</small><p>${esc(q.text)}</p>`).join('')}${e.completionConfirmed?`<small>结果</small><p>${esc(e.result)}</p>`:''}</div></details>`).join('')}</section>`;
 }
 function show(){
  const child=contexts(opened.parent).find(c=>c.contextID===opened.context);
  if(!child){opened=null;toast('这段专家历史已不可用。');return;}
  const parent=conversations.find(c=>c.id===opened.parent),exchanges=records(opened.parent,child.contextID);
  const messages=child.messages||parent.messages.filter(m=>m.role==='user'||m.role===child.displayName);
  const peerInputs=exchanges.filter(e=>e.target.contextID===child.contextID).map(e=>({role:'expert',author:e.source.name,text:e.text,exchangeID:e.exchangeID}));
  openDialog(child.displayName+' · 专家会话',`<div class="eh-reader" data-expert-history="${esc(child.contextID)}"><p class="dialog-note">${child.isCurrent?'当前专家上下文':'旧配置快照'} · 只读 · 合成记录</p>${exchangeRows(exchanges)}<div class="eh-transcript">${[...messages,...peerInputs].map(m=>`<article${m.exchangeID?` data-peer-input="${esc(m.exchangeID)}"`:''}><small>${m.author?esc(m.author):m.role==='user'?'你':esc(child.displayName)}</small><p>${esc(m.text)}</p></article>`).join('')||'<p>还没有消息。</p>'}</div><div class="dialog-actions">${child.isCurrent?'<button type="button" class="primary" data-action="eh:continue">回到主会话继续</button>':''}<button type="button" data-action="close">完成</button></div></div>`);
 }
 function handle(a){if(!a.startsWith('eh:'))return false;const [kind,p,c]=a.slice(3).split(':');if(kind==='toggle'){const parent=decodeURIComponent(p);expanded.has(parent)?expanded.delete(parent):expanded.add(parent);render();}if(kind==='open'||kind==='inspect'){opened={parent:decodeURIComponent(p),context:decodeURIComponent(c)};show();}if(kind==='continue'&&opened){const value=opened;if(expertRolesModule.continueChild(value.parent,value.context)){opened=null;closeDialog();if(architectureModule.getMode()==='desktop')architectureModule.handle('dk-select:'+value.parent);else navigate('chat',value.parent);document.querySelector('#compose')?.focus();}}return true;}
 return {rows,handle,snapshot:()=>({expanded:[...expanded],opened,exchanges:structuredClone([...examples.values()].flat())})};
})();
