import test from 'node:test';
import assert from 'node:assert/strict';
import { editResendText,parseEditedResendEnvelope,confirmEditedResendAdmission,prepareEditedResend,editedResendSourceForMessage } from './editedResend.ts';
import { editedFixture } from './editedResend.fixture.ts';
import type { EditedResendEnvelope } from './editedResend.ts';
import {createChatTimeline,reduceChatTimeline,timelineRequest,type TimelineMessage} from './chatTimeline.ts';

test('edit captures source and keeps unchanged retry identity while deliberate text changes create a new intent',()=>{
    const {draft}=editedFixture();
    assert.equal(editResendText(draft,'Edited'),draft);
    const changed=editResendText(draft,'Changed');
    assert.notEqual(changed.input.operation_id,draft.input.operation_id);
    assert.notEqual(changed.input.target_conversation_id,draft.input.target_conversation_id);
    assert.equal(changed.source.originalText,'Original');
    assert.equal(changed.input.expected_source_revision,4);
    assert.equal(draft.input.edited_text,'Edited');
});
test('preparation sends only the captured public intent and cannot claim admission',async()=>{
    const {draft,envelope,receipt}=editedFixture();let sent:Record<string,unknown>|undefined;
    const result=await prepareEditedResend(draft,(async(path:string,options?:RequestInit)=>{
        assert.equal(path,'/api/chat/edited-resend/prepare');sent=JSON.parse(String(options?.body));return envelope;
    }) as never);
    assert.deepEqual(Object.keys(sent!).sort(),['client_id','expected_host_id','input']);
    assert.deepEqual(sent!.input,draft.input);
    assert.throws(()=>confirmEditedResendAdmission(draft,result,receipt),/not been admitted/);
    result.prepared.admitted_run_id=result.prepared.run_id;
    assert.equal(confirmEditedResendAdmission(draft,result,receipt).envelope.target.id,draft.input.target_conversation_id);
});
test('wrong Host, revisions, target, message IDs and receipt owner fail closed',()=>{
    const {draft,envelope,receipt}=editedFixture();
    for(const change of [
        (x:EditedResendEnvelope)=>x.host_id='other',
        (x:EditedResendEnvelope)=>x.prepared.input.expected_source_revision++,
        (x:EditedResendEnvelope)=>x.target.id='source',
        (x:EditedResendEnvelope)=>x.prepared.user_message_id='other-user',
    ]){const changed=structuredClone(envelope);change(changed);assert.throws(()=>parseEditedResendEnvelope(changed,draft.source,draft.input));}
    const admitted=structuredClone(envelope);admitted.prepared.admitted_run_id=admitted.prepared.run_id;
    assert.throws(()=>confirmEditedResendAdmission(draft,admitted,{...receipt,identity:{...receipt.identity,conversation_id:'source'}}));
});

function canonicalUserWitness(text='Exact original text'):TimelineMessage {
    const state=createChatTimeline({conversationId:'source',generation:1});
    const canonical={message_id:'source-user',role:'user',state:'completed',revision:7,created_at_ms:1,
        parts:[{kind:'text',part_id:'source-user:body',revision:3,markdown:text,is_final:true}]};
    const payload={sourceHostId:'host',conversationProjection:{contract_version:1,conversation_id:'source',revision:11,messages:[canonical]},
        messageWindow:{projectionRevision:11,eventSequence:12,before:null,hasMore:false,totalCount:1}};
    const result=reduceChatTimeline(state,{type:'history',request:timelineRequest(state),mode:'replace',payload});
    assert.equal(result.ignored,undefined);
    return result.state.messages[0];
}

test('edit source uses only the message-paired read snapshot and exact disclosed user body',()=>{
    const row=canonicalUserWitness(),owner={hostId:'host',clientId:'client',conversationId:'source'};
    const source=editedResendSourceForMessage(row,owner);
    assert.deepEqual(source,{...owner,conversationRevision:11,messageId:'source-user',messageRevision:7,originalText:'Exact original text'});
    assert.ok(Object.isFrozen(source));
    const noSnapshot={...row,canonicalSnapshot:null};
    assert.equal(editedResendSourceForMessage(noSnapshot,owner),null,'A canonical-looking row without a paired source read is not sufficient');
    for(const mutate of [
        (value:TimelineMessage)=>{value.canonicalSnapshot={...value.canonicalSnapshot!,sourceHostId:'foreign'};},
        (value:TimelineMessage)=>{value.canonicalSnapshot={...value.canonicalSnapshot!,conversationId:'foreign'};},
        (value:TimelineMessage)=>{value.canonicalSnapshot={...value.canonicalSnapshot!,revision:0};},
        (value:TimelineMessage)=>{value.canonicalSnapshot={...value.canonicalSnapshot!,revision:1.5};},
        (value:TimelineMessage)=>{value.canonical!.message_id='other';},
        (value:TimelineMessage)=>{value.canonical!.revision=0;},
        (value:TimelineMessage)=>{value.canonical!.role='assistant';},
        (value:TimelineMessage)=>{value.canonical!.state='pending';},
        (value:TimelineMessage)=>{value.state='pending';},
        (value:TimelineMessage)=>{const part=value.parts[0];if(part.source!=='canonical')throw Error('Expected real canonical fixture');value.parts=[{...part,value:{...part.value,markdown:'Different visible body'}}];},
        (value:TimelineMessage)=>{value.parts=[{...value.parts[0],id:'different-body'}];},
        (value:TimelineMessage)=>{value.canonical!.parts[0].review='pending';},
        (value:TimelineMessage)=>{value.canonical!.parts[0].is_final=false;},
        (value:TimelineMessage)=>{value.canonical!.parts[0].revision=0;},
        (value:TimelineMessage)=>{value.canonical!.parts.push({...value.canonical!.parts[0],part_id:'another'});},
    ]){
        const changed=structuredClone(row);mutate(changed);
        assert.equal(editedResendSourceForMessage(changed,owner),null);
    }
    const fresh=editedResendSourceForMessage(row,{...owner,conversationId:'another'});
    assert.equal(fresh,null);
});

test('attachment-encoded user messages remain ineligible even with genuine paired canonical revisions',()=>{
    const text='Question\n\n'+JSON.stringify({nola_web_text_attachments_version:1,documents:[{name:'file.txt',content:'Actual reference',clipped:false}]});
    const row=canonicalUserWitness(text);
    assert.equal(editedResendSourceForMessage(row,{hostId:'host',clientId:'client',conversationId:'source'}),null);
});

test('plain user text with opaque metadata stays editable only with its complete canonical body',()=>{
    const row=canonicalUserWitness('metadata-backed text');
    const part={kind:'historical',part_id:'source-user:body',revision:3,projection:{
        owner:{conversation_id:'source',message_id:'source-user',part_id:'source-user:body'},canonical_revision:11,
        canonical_kind:'text',schema:null,source:null,binding:null,status:{state:'unsupported',code:'schema_unrecognized'},fields:{},
        canonical_text:{text:'metadata-backed text',total_utf8_bytes:20},
    }};
    row.canonical!.parts=[part];row.parts[0].value=part;
    const owner={hostId:'host',clientId:'client',conversationId:'source'};
    assert.equal(editedResendSourceForMessage(row,owner)?.originalText,'metadata-backed text');
    part.projection.canonical_text.total_utf8_bytes=50;
    assert.equal(editedResendSourceForMessage(row,owner),null,'A truncated preview is not an edit witness');
    part.projection.canonical_text.total_utf8_bytes=20;part.projection.owner.message_id='foreign';
    assert.equal(editedResendSourceForMessage(row,owner),null);
});
