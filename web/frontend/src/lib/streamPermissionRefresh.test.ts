import {test} from 'node:test';
import assert from 'node:assert/strict';
import {createChatTurn, reduceChatTurn} from './chatTurn.ts';
import {refreshStreamPermissions} from './streamPermissionRefresh.ts';
const owner={conversationId:'c',hostId:'h',clientId:'client',generation:1};
const first={protocol_version:4,allow_conversation:false,nonce:'first',request_id:'call1',host_id:'h',client_id:'client',session_id:'c',canonical_run_id:'run',tool_name:'render_ui',expires_at:Math.floor(Date.now()/1000)+100};
const second={...first,nonce:'second',request_id:'call73',safe_review_text:'expected revision 2'};
const start=()=>{let turn=reduceChatTurn(createChatTurn(owner,'run'),{type:'run_bound',owner,runId:'run'}).state;turn=reduceChatTurn(turn,{type:'permission_pending',owner,challenge:first,nowSeconds:0}).state;return reduceChatTurn(turn,{type:'permission_acknowledged',owner,nonce:first.nonce}).state;};
const observation=(permissions:unknown[])=>({source_host_id:'h',conversation_id:'c',client_id:'client',goal_control:null,result:{protocol_version:1,conversation_id:'c',client_id:'client',run_id:'run',assistant_message_id:'run:assistant',status:'running',worker:{state:'running',generation:'1'},recovery:'observe',admission:null,terminal_seal:null,permissions,can_submit:false,submission_receipt:null,goal_execution:null,context_usage:null}});
test('SSE successor needs authoritative refresh and clears old accepted state',async()=>{
 let turn=start();assert.equal(reduceChatTurn(turn,{type:'permission_pending',owner,challenge:second,nowSeconds:0}).ignored,'permission');
 let resolve!:()=>void;const done=new Promise<void>(r=>resolve=r);
 const observer=refreshStreamPermissions(turn,async body=>{assert.equal(body.expected_run_id,'run');assert.equal(body.client_id,'client');return observation([second]);},()=>turn,(next,challenge)=>{turn=next;assert.equal(challenge?.safe_review_text,'expected revision 2');resolve();},()=>assert.fail('valid authority rejected'));
 await done;observer?.stop();assert.equal(turn.permission?.nonce,'second');assert.equal(turn.permissionAcknowledged,false);assert.equal(turn.phase,'awaiting_permission');
});
test('late snapshot cannot overwrite a newer challenge or switched view',async()=>{
 for(const changed of ['nonce','generation']){let turn=start();let deliver!:(v:unknown)=>void;let requested!:()=>void;const begun=new Promise<void>(r=>requested=r);let accepted=false;
 const observer=refreshStreamPermissions(turn,async()=>{requested();return new Promise(r=>deliver=r);},()=>turn,()=>{accepted=true;},()=>assert.fail('stale refresh reported failure'));
 await begun;turn=changed==='nonce'?reduceChatTurn(turn,{type:'permissions_observed',owner,challenges:[second],nowSeconds:0}).state:{...turn,owner:{...owner,generation:2}};
 deliver(observation([first]));await new Promise(r=>setTimeout(r,0));observer?.stop();assert.equal(accepted,false);}
});
