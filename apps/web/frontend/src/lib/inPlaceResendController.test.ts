import test from 'node:test';
import assert from 'node:assert/strict';
import { createConversationSubmissionController } from './conversationSubmissionController.ts';
import { SourceSubmissionStorage } from './sourceSubmission.ts';
import { inPlaceInput } from './inPlaceResend.ts';
import { editedFixture } from './editedResend.fixture.ts';

for (const admitted of [true,false]) test(`in-place resend retains prefix, owner and composer while requiring admission (receipt: ${admitted})`,async()=>{
    const {draft,receipt:original}=editedFixture(), target=draft.source.conversationId;
    const input=inPlaceInput(draft);
    const envelope={host_id:'host',client_id:'client',target:{id:target,title:'Original title',revision:2,configuration_revision:1},
        prepared:{input,run_id:input.operation_id,user_message_id:`${input.operation_id}:user`,experts:[]}};
    const receipt={...original,identity:{...original.identity,conversation_id:target},admission:original.admission?{...original.admission,
        identity:{...original.admission.identity,conversation_id:target}}:null};
    const data=new Map<string,string>();
    const storage=new SourceSubmissionStorage({get length(){return data.size;},key:i=>[...data.keys()][i]??null,
        getItem:k=>data.get(k)??null,setItem:(k,v)=>{data.set(k,v);},removeItem:k=>{data.delete(k);}});
    const drafts=new Map([[target,'Unsent original draft']]);
    let dispatch:Record<string,unknown>|undefined,planRequest:Record<string,unknown>|undefined;
    let end!:()=>void;const held=new Promise<void>(resolve=>end=resolve);
    const workspace={load:async()=>{},isLoaded:()=>true,restoreSubmissions:async()=>[],
        stageSubmission:async()=>assert.fail('Edit must not capture composer tickets'),
        updateSubmission:async()=>assert.fail('Edit must not update composer tickets'),
        retireSubmission:async()=>assert.fail('Edit must not consume composer tickets'),abandonSubmission:async()=>{},
        text:{read:(id:string)=>drafts.get(id)??''},attachments:{read:()=>[]},submissions:new Map(),replyNotices:new Map(),knowledge:new Map(),
        transports:new Set(),completions:{watch:async()=>{}},sources:{watch:async()=>{}}};
    const api=async(path:string,options?:RequestInit)=>{
        if(path.includes('/queued-input'))return {head:null,dispatch:null};
        if(path.includes('/messages'))return {sourceHostId:'host',conversationProjection:{contract_version:1,conversation_id:target,revision:2,messages:[{message_id:'prefix-user',role:'user',state:'completed',revision:2,created_at_ms:1,parts:[{kind:'text',part_id:'prefix:text',revision:2,markdown:'Keep this earlier question',is_final:true}]}],surface:{revision:2,committed_message_ids:['prefix-user'],live_message_id:null}},
            messageWindow:{projectionRevision:2,eventSequence:0,before:null,hasMore:false,totalCount:1},assistantJournalHeads:[]};
        const body=JSON.parse(String(options?.body??'{}'));
        if(path.endsWith('/observe'))return {source_host_id:'host',conversation_id:target,client_id:'client',goal_control:null,result:null};
        if(path==='/api/agent/tool-plan/prepare'){
            planRequest=body;return {source_host_id:'host',result:{schema_version:1,reference:{manifest_id:'manifest',host_id:'host',client_id:'client',session_id:target,catalog_revision:'d'.repeat(64)},issued_at_ms:1,expires_at_ms:60001,tools:[]}};
        }
        if(path.endsWith('/submission'))return {source_host_id:'host',result:{identity:receipt.identity,request_digest:receipt.request_digest}};
        throw new Error('Unexpected read in isolated protocol fixture');
    };
    const controller=createConversationSubmissionController({conversation:{conversationId:target,executionBackend:'nola',configurationRevision:1} as never,
        hostId:'host',clientId:'client',storage,workspace:workspace as never,request:api as never,recoveryOwners:async()=>[],
        stream:(async function*(_path:string,options:RequestInit){dispatch=JSON.parse(String(options.body));
            if(admitted){yield {type:'submission_receipt',receipt};await held;}else{yield {type:'status',state:'stream_opened'};}
        }) as never});
    try{
        const submission=controller.submitInPlaceResend(envelope,'en');
        if(admitted)assert.equal((await submission).phase,'admitted');
        else{await assert.rejects(submission,/unconfirmed/);assert.equal(storage.read('host','client',target)?.sent,true);}
        assert.deepEqual(planRequest,{expected_host_id:'host',client_id:'client',session_id:target});
        assert.equal(dispatch!.submission_id,envelope.prepared.run_id);
        assert.equal(dispatch!.user_message_id,envelope.prepared.user_message_id);
        assert.equal(dispatch!.assistant_message_id,`${input.operation_id}:assistant`);
        assert.equal(dispatch!.in_place_resend_operation_id,draft.input.operation_id);
        assert.deepEqual(dispatch!.messages,[{role:'user',content:'Edited'}]);
        assert.equal(dispatch!.expected_parent_message_id,'prefix-user');
        for(const forbidden of ['model_intent','modelProfileId','model','provider','endpoint','api_key','core_config','attachments','host_attachments','context_files','queued_user_command_id'])assert.equal(Object.hasOwn(dispatch!,forbidden),false,forbidden);
        assert.equal(drafts.get('source'),'Unsent original draft');
        assert.equal(drafts.get(target),'Unsent original draft');
    }finally{end();controller.dispose();}
});
