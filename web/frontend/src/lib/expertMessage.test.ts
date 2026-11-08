// Protocol fixture coverage only: real Host/model/UI verification runs separately.
import test from 'node:test';
import assert from 'node:assert/strict';
import { editedFixture } from './editedResend.fixture.ts';
import { admitExpertMessage, completeEditedExpertResend, loadExpertManifest, saveExpertIntent, loadExpertIntent, clearExpertIntent, confirmEditedExpertResendCompletion, resumePreparedExpertMessage } from './expertMessage.ts';
import type { ExpertMessageEntry, AdmitExpertMessageInput } from './expertMessage.ts';

function fixture(){
    const base=editedFixture(),{envelope,receipt}=base;
    const draft={source:base.draft.source,input:{...base.draft.input,operation_id:'55555555-5555-4555-8555-555555555555',target_conversation_id:'66666666-6666-4666-8666-666666666666'}};
    const sourceRun='77777777-7777-4777-8777-777777777777';
    envelope.prepared.input.source_conversation_id='source-context';envelope.prepared.input.source_message_id=`${sourceRun}:user`;
    Object.assign(envelope.prepared.source,{conversation_id:'source-context',message_id:`${sourceRun}:user`,run_id:sourceRun,assistant_message_id:`${sourceRun}:assistant`});
    const participant={participant_id:'member',agent_id:'expert',agent_revision:2,context_conversation_id:envelope.target.id};
    const originalParticipant={...participant,participant_id:'old-member',context_conversation_id:'source-context'};
    const original:ExpertMessageEntry={participant:originalParticipant,display_name:'Expert',description:'',defaults:{},
        model_declaration:{profile_id:'profile',profile_revision:1,provider:'http',model_name:'model',runtime:'cloud',parameters:{}},
        context_configuration_revision:1,child_submission_id:sourceRun,child_user_message_id:`${sourceRun}:user`,child_assistant_message_id:`${sourceRun}:assistant`,
        expected_prompt_definition:null,model_witness:structuredClone(envelope.prepared.source),instruction_witness:{checkpoint_sha256:'d'.repeat(64),component_payload_digest:'e'.repeat(64),prompt_definitions:[]}};
    const input:AdmitExpertMessageInput={conversation_id:'source',expected_revision:4,room_id:'room',expected_room_revision:2,
        submission_id:'source-submission',user_message_id:'source-user',text:'Original',selected_participants:[originalParticipant],created_at_ms:100};
    const prepared={input:{...draft.input},source_submission_id:input.submission_id,room_id:'new-room',parent_message_id:`${draft.input.operation_id}:user`,
        entries:[{original,participant,prepared:structuredClone(envelope.prepared)}]};
    const expert={host_id:'host',client_id:'client',prepared,target:{...envelope.target,id:draft.input.target_conversation_id}};
    const source={host_id:'host',client_id:'client',manifest:{version:1,input,entries:[original]}};
    envelope.client_id='room:new-room';receipt.identity.client_id='room:new-room';
    return {draft,envelope,receipt,input,expert,source};
}
test('source absence is an explicit nullable read; actual Host errors are not treated as ordinary source',async()=>{
    const {draft}=fixture();
    assert.equal(await loadExpertManifest(draft.source,(async()=>({host_id:'host',client_id:'client',manifest:null}))as never),null);
    await assert.rejects(loadExpertManifest(draft.source,(async()=>{throw new Error('store unavailable');})as never),/store unavailable/);
});
test('complete-set admit tolerates object property ordering but rejects a partial manifest',async()=>{
    const {input}=fixture();let commands=0;
    const original=fixture().expert.prepared.entries[0].original;
    const response={host_id:'host',client_id:'client',manifest:{version:1,input:{...input},entries:[original]}};
    const admitted=await admitExpertMessage(input,{hostId:'host',clientId:'client'},(async(_path:string,options:RequestInit)=>{
        commands++;assert.deepEqual(Object.keys(JSON.parse(String(options.body))).sort(),['client_id','expected_host_id','input']);return response;
    })as never);
    assert.equal(commands,1);assert.equal(admitted.entries.length,1);
    await assert.rejects(admitExpertMessage(input,{hostId:'host',clientId:'client'},(async()=>({...response,manifest:{...response.manifest,entries:[]}}))as never),/Incomplete/);
});
test('expert edited send uses fixed ordinary child IDs and confirms both receipt and durable full plan',async()=>{
    const {draft,envelope,receipt,expert,source}=fixture();let dispatches=0,admitted=false;
    const request=(async(path:string)=>{
        if(path.includes('/expert-messages/'))return source;
        if(path==='/api/chat/edited-expert-resend/prepare')return expert;
        if(path.includes('/edited-expert-resend?')){
            const result=structuredClone(expert);result.prepared.entries[0].prepared.admitted_run_id=admitted?envelope.prepared.run_id:null;return result;
        }
        if(path.includes('/edited-resend?')){assert.equal(new URL(path,'http://fixture').searchParams.get('client_id'),'room:new-room');const result=structuredClone(envelope);result.prepared.admitted_run_id=admitted?result.prepared.run_id:null;return result;}
        throw new Error('unexpected endpoint');
    })as never;
    const completed=await completeEditedExpertResend(draft,{request,language:'en',controller:async actual=>({
        submitPreparedEditedResend:async received=>{assert.deepEqual(actual,received);assert.equal(received.client_id,'room:new-room');dispatches++;admitted=true;return receipt;},
    })});
    assert.equal(dispatches,1);assert.equal(completed.receipts.length,1);
});
test('stream/prepare without durable child admission remains partial and preserves original intent',async()=>{
    const {draft,envelope,receipt,expert,source}=fixture();const original=structuredClone(draft.input);
    await assert.rejects(completeEditedExpertResend(draft,{language:'en',request:(async(path:string)=>path.includes('/expert-messages/')?source:path.includes('edited-expert')?expert:envelope)as never,
        controller:async()=>({submitPreparedEditedResend:async()=>receipt})}),/partial \(0\/1\)/);
    assert.deepEqual(draft.input,original);
});

test('uncertain whole expert intent persists exact revisions and selected collection across UI remount, isolated by Host/client',()=>{
    const {input}=fixture();const data=new Map<string,string>();
    const storage={getItem:(key:string)=>data.get(key)??null,setItem:(key:string,value:string)=>{data.set(key,value);},removeItem:(key:string)=>{data.delete(key);}};
    const owner={hostId:'host',clientId:'client'};
    saveExpertIntent(storage,owner,input);
    assert.deepEqual(loadExpertIntent(storage,owner,input.conversation_id),input);
    assert.equal(loadExpertIntent(storage,{...owner,hostId:'other'},input.conversation_id),null);
    assert.equal(loadExpertIntent(storage,{...owner,clientId:'other'},input.conversation_id),null);
    assert.throws(()=>clearExpertIntent(storage,owner,{...input,text:'replacement'}),/different expert intent/);
    assert.deepEqual(loadExpertIntent(storage,owner,input.conversation_id)?.selected_participants,input.selected_participants);
    clearExpertIntent(storage,owner,input);assert.equal(data.size,0);
});

test('expert child completion rejects browser-client receipts instead of inheriting parent permission ownership',()=>{
    const {draft,expert,receipt}=fixture();expert.prepared.entries[0].prepared.admitted_run_id=expert.prepared.entries[0].prepared.run_id;
    assert.doesNotThrow(()=>confirmEditedExpertResendCompletion(draft,{envelope:expert,receipts:[receipt]}));
    const foreign=structuredClone(receipt);foreign.identity.client_id='client';
    assert.throws(()=>confirmEditedExpertResendCompletion(draft,{envelope:expert,receipts:[foreign]}),/Invalid source submission/);
});
test('explicit prepared expert recovery retains the durable original operation and uses the new Room client',async()=>{
    const {draft,expert,envelope,receipt,source,input}=fixture();let admitted=false,preparedInput:unknown;
    const entry=expert.prepared.entries[0];
    const visible={version:1,input:{...input,conversation_id:expert.target.id,submission_id:draft.input.operation_id,
        user_message_id:expert.prepared.parent_message_id,text:draft.input.edited_text,selected_participants:[entry.participant]},
        entries:[{...entry.original,participant:entry.participant,child_submission_id:entry.prepared.run_id,
            child_user_message_id:entry.prepared.user_message_id,child_assistant_message_id:entry.prepared.assistant_message_id}]};
    const request=(async(path:string,options?:RequestInit)=>{
        if(path.includes('/expert-messages/'))return source;
        if(path==='/api/chat/edited-expert-resend/prepare'){preparedInput=JSON.parse(String(options?.body)).input;return expert;}
        if(path.includes('/edited-expert-resend?')){const current=structuredClone(expert);current.prepared.entries[0].prepared.admitted_run_id=admitted?entry.prepared.run_id:null;return current;}
        if(path.includes('/edited-resend?')){assert.equal(new URL(path,'http://fixture').searchParams.get('client_id'),'room:new-room');const current=structuredClone(envelope);current.prepared.admitted_run_id=admitted?entry.prepared.run_id:null;return current;}
        throw new Error('unexpected endpoint');
    })as never;
    assert.equal(await resumePreparedExpertMessage(visible,{hostId:'host',clientId:'client'},{request,language:'en',
        controller:async()=>({submitPreparedEditedResend:async()=>{admitted=true;return receipt;}})}),true);
    assert.deepEqual(preparedInput,draft.input);
});
