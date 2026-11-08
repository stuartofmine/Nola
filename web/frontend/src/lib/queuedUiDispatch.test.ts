import test from 'node:test';
import assert from 'node:assert/strict';
import { queuedUiDispatch } from './queuedUiDispatch.ts';
const owner = {hostId:'host',clientId:'client',conversationId:'conversation',generation:1};
const head = {command_id:'action',kind:'user_input',user_message_id:'ui:action:user',lease_until_ms:null};
const receipt = {protocol_version:1,identity:{source_host_id:'host',client_id:'client',conversation_id:'conversation',
    submission_id:'action',user_message_id:'ui:action:user',assistant_message_id:'action:assistant'},
    request_digest:'a'.repeat(64),phase:'rejected',admission:null,rejection_code:'dispatch_not_admitted'};
test('queued action retains exact rejected receipt instead of creating a new request', () => {
    assert.deepEqual(queuedUiDispatch(receipt,head,owner),receipt);
    assert.equal(queuedUiDispatch(null,head,owner),null);
    assert.throws(()=>queuedUiDispatch(undefined,head,owner));
    assert.throws(()=>queuedUiDispatch({...receipt,request_digest:'new-request'},head,owner));
});
test('queued receipt is fenced to source client conversation and logical action', () => {
    for(const key of ['hostId','clientId','conversationId'] as const)
        assert.throws(()=>queuedUiDispatch(receipt,head,{...owner,[key]:'other'}));
    assert.throws(()=>queuedUiDispatch(receipt,{...head,command_id:'other'},owner));
    assert.throws(()=>queuedUiDispatch({...receipt,phase:'admitted'},head,owner));
});
