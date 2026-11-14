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
