// SPDX-License-Identifier: Apache-2.0
// Opt-in production-client + real Host/Store/model acceptance. This is API
// integration evidence, not browser-click or pixel evidence.
import assert from 'node:assert/strict';
import {readFileSync, writeFileSync} from 'node:fs';
import {parseChatConversation} from '../frontend/src/lib/chatConversations.ts';
import {parseSessionProfiles} from '../frontend/src/lib/chatSessionSettings.ts';
import {createWorkspaceDrafts} from '../frontend/src/lib/workspaceDrafts.ts';
import {createConversationSubmissionController} from '../frontend/src/lib/conversationSubmissionController.ts';
import type {ConversationSubmissionSnapshot} from '../frontend/src/lib/conversationSubmissionController.ts';
import {canReleaseChatTurn} from '../frontend/src/lib/chatTurn.ts';
import {timelineMessageText} from '../frontend/src/lib/chatTimeline.ts';
import {createEditedResendDraft, editedResendSourceForMessage, completeEditedResend} from '../frontend/src/lib/editedResend.ts';
import {admitExpertMessage, completeEditedExpertResend, loadExpertManifest} from '../frontend/src/lib/expertMessage.ts';
import {createRoomReplyProjection} from '../frontend/src/lib/roomReplyProjection.ts';
import {RunObserver} from '../frontend/src/lib/runObservation.ts';
import {api} from '../frontend/src/lib/api.ts';
import {chatEventStream} from '../frontend/src/lib/chatTransport.ts';

assert.equal(process.env.NOLA_RUN_REAL_MODEL_E2E, '1', 'Real model gate must be explicitly enabled');
const [origin, tokenFile, reportFile] = process.argv.slice(2);
assert.ok(origin && tokenFile && reportFile, 'Usage: <isolated localhost origin> <Host token file> <report path>');
const base = new URL(origin);
assert.equal(base.protocol, 'http:');
assert.equal(base.hostname, '127.0.0.1');
const nativeFetch = globalThis.fetch;
const cookies = new Map<string,string>();
const memory = new Map<string,string>();
Object.defineProperty(globalThis, 'localStorage', {value: {
    get length(){return memory.size;}, key:(i:number)=>[...memory.keys()][i]??null,
    getItem:(key:string)=>memory.get(key)??null, setItem:(key:string,value:string)=>{memory.set(key,value);},
    removeItem:(key:string)=>{memory.delete(key);},
}, configurable:true});
// Only the production HTTP adapter's cookie/visibility inputs are supplied.
// No DOM, component event, receipt, model output, or Store result is fabricated.
Object.defineProperty(globalThis, 'document', {value:{hidden:false,
    get cookie(){return [...cookies].map(([key,value])=>`${key}=${value}`).join('; ');}}, configurable:true});
globalThis.fetch = async (input, init={}) => {
    const url = new URL(typeof input==='string'?input:input instanceof URL?input.href:input.url, base);
    assert.equal(url.origin, base.origin, 'Session credentials must stay on the isolated Host');
    const headers = new Headers(init.headers);
    headers.set('Origin',base.origin);
    if(cookies.size)headers.set('Cookie',[...cookies].map(([key,value])=>`${key}=${value}`).join('; '));
    const response = await nativeFetch(url,{...init,headers});
    for(const value of response.headers.getSetCookie()){
        const first=value.split(';',1)[0], at=first.indexOf('=');
        if(at>0)cookies.set(first.slice(0,at),first.slice(at+1));
    }
    return response;
};
const expertOnly=process.env.NOLA_REAL_ONLY_EXPERTS==='1';
const report={kind:'real-host-production-client-api',scope:expertOnly?'experts-only':'all',passed:[] as string[],failures:[] as string[],failed:false};
function passed(value:string){report.passed.push(value);console.log('passed',value);}
function failure(stage:string,error:unknown){
    report.failed=true;
    const reason=String(error).replace(/https?:\/\/[^\s"<>]+/g,'<endpoint>');
    report.failures.push(`${stage}: ${reason}`);console.error(stage,reason);
}
const workspaces=createWorkspaceDrafts(null);
const controllers=new Map<string,ReturnType<typeof createConversationSubmissionController>>();
const snapshots=new Map<string,ConversationSubmissionSnapshot>();
const clientId=`real-workspace-${crypto.randomUUID()}`;
let hostId='';
const delay=(ms:number)=>new Promise(resolve=>setTimeout(resolve,ms));
async function until<T>(read:()=>T|undefined|false, label:string, timeout=180_000):Promise<T>{
    const end=Date.now()+timeout;
    while(Date.now()<end){const value=read();if(value)return value;await delay(100);}
    throw new Error(`Timed out: ${label}`);
}
async function controller(id:string, client=clientId){
    const existing=controllers.get(id);
    if(existing){assert.equal(existing.owner.clientId,client);return existing;}
    const conversation=parseChatConversation(await api(`/api/chat/conversations/${encodeURIComponent(id)}`));
    const value=createConversationSubmissionController({conversation,hostId,clientId:client,
        stream:async function*(path,options){
            for await(const event of chatEventStream(path,options)){
                if(event.type==='error')console.log('host-error',String(event.message??event.error??event.code).replace(/https?:\/\/[^\s"<>]+/g,'<endpoint>'));
                yield event;
            }
        },
        workspace:workspaces.forHost(hostId),recoveryOwners:async()=>[]});
    controllers.set(id,value);value.attach(snapshot=>{
        snapshots.set(id,snapshot);
        if(snapshot.error)console.log('production-client-error',snapshot.error.replace(/https?:\/\/[^\s"<>]+/g,'<endpoint>'));
    });
    await value.initialize();value.observe();return value;
}
async function settled(id:string, marker:string){
    await until(()=>{
        const snapshot=snapshots.get(id);
        if(!snapshot?.turn||!canReleaseChatTurn(snapshot.turn))return;
        return snapshot;
    },'canonical terminal and real response');
    await controllers.get(id)!.loadHistory('replace');
    const snapshot=snapshots.get(id)!;
    const text=snapshot.timeline?.messages.filter(m=>m.role==='assistant').map(timelineMessageText).join('\n')??'';
    assert.ok(text.includes(marker),`Real sealed response must contain its requested marker; outcome=${snapshot.turn?.outcome}; fixture response=${text.slice(0,500)}`);
    return snapshot;
}
async function newConversation(){
    const raw=await api<Record<string,unknown>>('/api/chat/conversations',{method:'POST',body:'{}'});
    return parseChatConversation(raw);
}
async function source(id:string){
    const value=await controller(id);await value.loadHistory('replace');
    const snapshot=snapshots.get(id)!;
    const message=snapshot.timeline?.messages.find(m=>m.role==='user');assert.ok(message);
    const result=editedResendSourceForMessage(message,{hostId,clientId,conversationId:id,
        conversationRevision:snapshot.timeline!.projection!.revision});assert.ok(result);return result;
}
async function expertTerminal(roomId:string, context:string, run:string, marker:string){
    const owner={hostId,clientId:`room:${roomId}`,conversationId:context,generation:1,
        runId:run,assistantMessageId:`${run}:assistant`};
    let output='';const projection=createRoomReplyProjection(owner,api,message=>{output=timelineMessageText(message);});
    const observer=new RunObserver();let terminal=false, exhausted:unknown;
    const timeout=setTimeout(()=>observer.stop(),180_000);
    try{
        await observer.run(owner,(body,signal)=>api('/api/chat/agent/v2/observe',{method:'POST',signal,body:JSON.stringify(body)}),
            async observation=>{
                if(!observation)return true;
                assert.equal(observation.run_id,run);
                if(!await projection.accept(observation)){assert.ok(output.includes(marker));terminal=true;return false;}
                return true;
            },error=>{exhausted=error;},{run_id:run,assistant_message_id:owner.assistantMessageId});
        if(exhausted)throw exhausted;
        assert.ok(terminal,'Timed out: exact expert terminal');
    }finally{clearTimeout(timeout);observer.stop();projection.dispose();}
}
try{
    await api('/api/web/login',{method:'POST',body:JSON.stringify({token:readFileSync(tokenFile,'utf8').trim()})});
    const session=await api<{authenticated:boolean;hostId:string}>('/api/web/session');
    assert.equal(session.authenticated,true);hostId=session.hostId;
    if(!expertOnly){
    const ordinary=await newConversation(), ordinaryController=await controller(ordinary.conversationId);
    await until(()=>snapshots.get(ordinary.conversationId)?.observedCanSubmit,'new conversation authority');
    await ordinaryController.submit({text:'Reply only WEB_REAL_ORIGINAL_OK',attachments:[],language:'en'});
    await settled(ordinary.conversationId,'WEB_REAL_ORIGINAL_OK');
    passed('ordinary-real-model-and-verified-terminal');
    try {
    const original=await source(ordinary.conversationId);
    const edited=createEditedResendDraft(original,'Reply only WEB_REAL_EDIT_OK');
    const completion=await completeEditedResend(edited,{language:'en',controller:e=>controller(e.target.id,e.client_id)});
    assert.equal(completion.receipt.phase,'admitted');
    await settled(completion.envelope.target.id,'WEB_REAL_EDIT_OK');
    assert.equal((await source(ordinary.conversationId)).originalText,original.originalText);
    const newMessages=snapshots.get(completion.envelope.target.id)!.timeline!.messages.filter(m=>m.role==='user');
    assert.equal(newMessages.length,1);assert.equal(timelineMessageText(newMessages[0]),edited.input.edited_text);
    passed('ordinary-edit-new-owner-original-preserved-no-history-prefix');
    } catch(error){failure('ordinary-edit',error);}
    }

    // Real Core directory/Room commands and existing Host dispatcher; no child
    // is synthesized and no successful subset substitutes for the full plan.
    try {
    const profiles=parseSessionProfiles(await api('/api/model-profiles'));
    const profile=profiles.find(p=>p.selected&&p.enabled);assert.ok(profile,'An isolated canonical default profile is required');
    const roomAPI=async <T>(command:unknown):Promise<T>=>{
        console.log('room-command',(command as {type?:string}).type);
        return api<T>('/api/agent-rooms',{method:'POST',body:JSON.stringify(command)});
    };
    let directory=await roomAPI<{revision:number|null;agents:{id:string;revision:number}[]}>({type:'directory'});
    const expertIds=new Set<string>();
    for(let i=0;i<2;i++){
        const expertId=`real-expert-${i}-${crypto.randomUUID()}`;expertIds.add(expertId);
        directory=await roomAPI({type:'configure_agent',input:{
        id:expertId,expected_directory_revision:directory.revision,expected_agent_revision:null,
        name:`Real expert ${i+1}`,description:'Returns the requested short marker.',
        defaults:{modelSelectionID:`configured:${profile.id}`},availability:'active',updated_at_ms:Date.now()}});
    }
    const parent=await newConversation();
    let association=await roomAPI<{room:{revision:number;room:{id:string;participants:any[]}}}>({type:'ensure_for_conversation',conversation_id:parent.conversationId,at_ms:Date.now()});
    let room=association.room;
    for(const agent of directory.agents.filter(a=>expertIds.has(a.id)))room=await roomAPI({type:'join',input:{room_id:room.room.id,
        expected_room_revision:room.revision,agent_id:agent.id,expected_agent_revision:agent.revision,at_ms:Date.now()}});
    const current=parseChatConversation(await api(`/api/chat/conversations/${encodeURIComponent(parent.conversationId)}`));
    const id=crypto.randomUUID();
    console.log('expert-admit');
    const plan=await admitExpertMessage({conversation_id:parent.conversationId,expected_revision:current.revision,
        room_id:room.room.id,expected_room_revision:room.revision,submission_id:id,user_message_id:`${id}:user`,
        text:'Reply only WEB_REAL_EXPERT_OK',created_at_ms:Date.now(),selected_participants:room.room.participants.map(p=>({
            participant_id:p.id,agent_id:p.agent_id,agent_revision:p.agent_revision,context_conversation_id:p.context_conversation_id}))},{hostId,clientId});
    assert.equal(plan.entries.length,2);
    for(const child of plan.entries){
        const receipt=await roomAPI<{input:unknown}>({type:'submission',room_id:room.room.id,submission_id:child.child_submission_id});
        const delivered=await roomAPI<{state:string;failure_code:string|null}>({type:'submit',input:receipt.input});
        assert.ok(['accepted','selected','executing','completed'].includes(delivered.state),`Expert source dispatch ${delivered.state}: ${delivered.failure_code}`);
    }
    await Promise.all(plan.entries.map(e=>expertTerminal(plan.input.room_id,e.participant.context_conversation_id,e.child_submission_id,'WEB_REAL_EXPERT_OK')));
    passed('complete-original-expert-set-real-model-and-verified-terminals');
    const parentSource=await source(parent.conversationId);
    const captured=await loadExpertManifest(parentSource);assert.equal(captured?.entries.length,2);
    for(const entry of captured!.entries){
        assert.ok(entry.model_witness, 'Actual source Run must freeze its model witness');
        assert.ok(entry.instruction_witness, 'Actual source Run must freeze its typed instruction witness');
    }
    // Directory edits after execution must not change the source definition or
    // silently replace its expert set with the current active picker contents.
    directory=await roomAPI({type:'directory'});
    for(const agent of [...directory.agents].filter(a=>expertIds.has(a.id))){
        const currentAgent=directory.agents.find(a=>a.id===agent.id)!;
        directory=await roomAPI({type:'configure_agent',input:{id:agent.id,
            expected_directory_revision:directory.revision,expected_agent_revision:currentAgent.revision,
            name:'Changed after original send',description:'Current directory must not replace the frozen source.',
            defaults:{},availability:'archived',updated_at_ms:Date.now()}});
    }
    const expertDraft=createEditedResendDraft(parentSource,'Reply only WEB_REAL_EXPERT_EDIT_OK');
    const expertCompletion=await completeEditedExpertResend(expertDraft,{language:'en',controller:e=>controller(e.target.id,e.client_id)});
    assert.equal(expertCompletion.receipts.length,2);
    for(const entry of expertCompletion.envelope.prepared.entries){
        const original=captured!.entries.find(source=>source.child_submission_id===entry.original.child_submission_id);
        assert.ok(original);assert.deepEqual(entry.original,original);
        assert.notEqual(entry.participant.context_conversation_id,original.participant.context_conversation_id);
        assert.notEqual(entry.prepared.run_id,original.child_submission_id);
    }
    await Promise.all(expertCompletion.envelope.prepared.entries.map(e=>expertTerminal(expertCompletion.envelope.prepared.room_id,
        e.participant.context_conversation_id,e.prepared.run_id,'WEB_REAL_EXPERT_EDIT_OK')));
    assert.equal((await loadExpertManifest(parentSource))!.input.text,parentSource.originalText);
    passed('complete-expert-set-real-model-and-fresh-edited-children-after-directory-archive');
    }catch(error){failure('expert-set-and-edit',error);}
}catch(error){failure('setup-or-ordinary',error);}
finally{
    for(const value of controllers.values())value.dispose();workspaces.clear();
    globalThis.fetch=nativeFetch;writeFileSync(reportFile,JSON.stringify(report,null,2)+'\n');
    console.log(JSON.stringify(report));
    if(report.failed)process.exitCode=1;
}
