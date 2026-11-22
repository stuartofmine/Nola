import test from 'node:test';
import assert from 'node:assert/strict';
import { editedFixture } from './editedResend.fixture.ts';
import { inPlaceInput, prepareInPlaceResend, confirmInPlaceCompletion, type InPlaceResendEnvelope } from './inPlaceResend.ts';
test('rewind uses the existing conversation, exact revisions and stable retry operation',async()=>{
    const {draft,receipt}=editedFixture(),input=inPlaceInput(draft);
    assert.equal(input.conversation_id,draft.source.conversationId);
    assert.equal(input.operation_id,draft.input.operation_id);
    assert.equal(Object.hasOwn(input,'target_conversation_id'),false);
    const envelope:InPlaceResendEnvelope={host_id:draft.source.hostId,client_id:draft.source.clientId,
        target:{id:input.conversation_id,title:'Keep title',revision:6,configuration_revision:2},
        prepared:{input,run_id:input.operation_id,user_message_id:`${input.operation_id}:user`,experts:[]}};
    const response=await prepareInPlaceResend(draft,(async(path:string,options:RequestInit)=>{
        assert.equal(path,'/api/chat/in-place-resend/prepare');
        assert.deepEqual(JSON.parse(String(options.body)).input,input);return envelope;
    }) as never);
    const admitted={...receipt,identity:{...receipt.identity,conversation_id:input.conversation_id}};
    assert.doesNotThrow(()=>confirmInPlaceCompletion(draft,{inPlace:true,envelope:response,receipt:admitted}));
    assert.throws(()=>confirmInPlaceCompletion(draft,{inPlace:true,envelope:response}));
    await assert.rejects(prepareInPlaceResend(draft,(async()=>({...envelope,target:{...envelope.target,id:'new-branch'}})) as never));
    await assert.rejects(prepareInPlaceResend(draft,(async()=>({...envelope,host_id:'foreign'})) as never));
});

test('resend preparation verifies the exact edited expert identities before starting inference',async()=>{
    const {draft}=editedFixture(),text='请@测试复查';
    const selection={agent_id:'qa',agent_revision:3,participant_id:'m',context_conversation_id:'c'};
    const withMentions={...draft,input:{...draft.input,edited_text:text},mentions:[{start:1,text:'@测试',selection}],expertEpoch:'epoch'};
    const input=inPlaceInput(withMentions);
    assert.deepEqual(input.expert_mentions?.segments,[{text:'请'},{text:'@测试',agent_id:'qa'},{text:'复查'}]);
    assert.deepEqual(input.selected_experts,[selection]);assert.equal(input.expected_source_store_epoch,'epoch');
    const response={host_id:draft.source.hostId,client_id:draft.source.clientId,target:{id:input.conversation_id},prepared:{
        input:JSON.parse(JSON.stringify(input)),run_id:input.operation_id,user_message_id:input.operation_id+':user',experts:[]}};
    await prepareInPlaceResend(withMentions, (async()=>response) as never);
    response.prepared.input.selected_experts[0].agent_revision=4;
    await assert.rejects(prepareInPlaceResend(withMentions,(async()=>response) as never));
    assert.throws(()=>inPlaceInput({...withMentions,expertEpoch:undefined}));
});
