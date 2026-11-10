import { test } from 'node:test';
import assert from 'node:assert/strict';
import { chatEventStream } from './chatTransport.ts';
import { createChatTurn, reduceChatTurn, canReleaseChatTurn } from './chatTurn.ts';
import { createChatTimeline, reduceChatTimeline, timelineMessageText } from './chatTimeline.ts';
import { AssistantJournalReader } from './assistantJournal.ts';
import { terminalFixture } from './assistantTerminal.fixture.ts';
import { createSourceSubmission, bindSourceDigest, acceptSourceReceipt, sourceSubmissionBody, sourceIdentity } from './sourceSubmission.ts';

const owner = { conversationId: 'conversation', generation: 1, hostId: 'host', clientId: 'fixture-browser' };
const identity = { submissionId: 'submission', userMessageId: 'contract-user-1', assistantMessageId: 'contract-assistant-1' };
const obsoleteAggregate = { type: 'conversation_projection_committed_v1', session_id: owner.conversationId, source_host_id: owner.hostId, submission_id: identity.submissionId };
const source = sourceIdentity(owner, identity);
const request = { conversationId: owner.conversationId, expected_host_id: owner.hostId, client_id: owner.clientId,
    submission_id: identity.submissionId, user_message_id: identity.userMessageId, assistant_message_id: identity.assistantMessageId,
    messages: [{ role: 'user', content: 'original frozen input' }] };
const sourceReceipt = { type: 'submission_receipt', receipt: { protocol_version: 1, identity: source, request_digest: 'a'.repeat(64), phase: 'admitted',
    admission: { identity: { source_store_epoch: 'epoch', conversation_id: owner.conversationId, run_id: identity.submissionId,
        assistant_message_id: identity.assistantMessageId }, sequence: '1', record_digest: 'b'.repeat(64) }, rejection_code: null } };
const through = { identity: { source_store_epoch:'epoch', conversation_id:owner.conversationId,
    run_id:identity.submissionId, assistant_message_id:identity.assistantMessageId }, sequence:'1', record_digest:'d'.repeat(64) };
const live = { type:'assistant_journal_committed_v1', session_id:owner.conversationId, source_host_id:owner.hostId, through };
function liveReader() {
    const text='临时答复 👋'; const size=String(new TextEncoder().encode(text).length);
    return new AssistantJournalReader(owner.hostId,owner.conversationId,owner.generation,async request=>{
        let result:unknown;
        switch(request.operation) {
            case 'open_snapshot': result={snapshot_id:'live',at:through}; break;
            case 'parts': result={at:through,next_part_id:null,parts:[{identity:through.identity,part_id:'fixture-live-text',generation:'0',revision:'1',metadata_revision:'1',
                order_key:{section:'transcript',position:'0'},metadata:{kind:'text',data:{visibility:'public',review:null,is_final:false}},
                bodies:[{field:'markdown',content_generation:'0',utf8_bytes:size,chunk_count:'1',content_root:'e'.repeat(64)}]}]}; break;
            case 'range': result={offset_bytes:'0',next_offset_bytes:size,total_utf8_bytes:size,text,end_of_content:true};break;
            case 'release_snapshot':result=null;break;
            default:throw new Error('Unexpected live read '+request.operation);
        }
        return {source_host_id:owner.hostId,result};
    });
}

test('byte transport cannot release the turn until authenticated sealed heads materialize', async t => {
    Object.defineProperty(globalThis, 'document', { configurable: true, value: { cookie: '' } });
    t.after(() => Reflect.deleteProperty(globalThis, 'document'));
    const events = [sourceReceipt, live, live, { trace: { kind: 'run.completed' } }, obsoleteAggregate, obsoleteAggregate];
    const wire = events.map(event => `data:${JSON.stringify(event)}\r\n\r\n`).join('') + 'data:[DONE]\n\n';
    t.mock.method(globalThis, 'fetch', async () => new Response(new ReadableStream({ start(controller) {
        for (const byte of new TextEncoder().encode(wire)) controller.enqueue(Uint8Array.of(byte));
        controller.close();
    } }), { headers: { 'Content-Type': 'text/event-stream' } }));
    let turn = createChatTurn(owner, identity);
    let pending = bindSourceDigest(createSourceSubmission(source, request), { source_host_id: owner.hostId,
        result: { identity: source, request_digest: 'a'.repeat(64) } });
    let timeline = reduceChatTimeline(createChatTimeline(owner), { type: 'beginSubmission', owner, submissionId: identity.submissionId }).state;
    const reader=liveReader();
    let count = 0;
    for await (const payload of chatEventStream('/api/chat/agent/v2/stream', { method: 'POST', body: JSON.stringify(sourceSubmissionBody(pending)) })) {
        if (payload.type === 'submission_receipt') {
            pending = acceptSourceReceipt(pending, payload.receipt);
            turn = reduceChatTurn(turn, { type: 'run_bound', owner, runId: pending.receipt!.admission!.identity.run_id }).state;
            continue;
        }
        const previousVersion = timeline.version;
        if(payload.type==='assistant_journal_committed_v1') {
            const update=await reader.notification(payload,turn.runId!);
            if(update) timeline=reduceChatTimeline(timeline,{type:'journal',owner,sourceHostId:owner.hostId,runId:turn.runId!,update}).state;
        }
        turn = reduceChatTurn(turn, { type: 'event', owner, payload }).state;
        count++;
        if (count < 4) {
            assert.equal(canReleaseChatTurn(turn), false);
            assert.equal(timelineMessageText(timeline.messages[0]), '临时答复 👋');
        }
        if ([2, 5].includes(count)) assert.equal(timeline.version, previousVersion, 'replay is not an append');
    }
    assert.equal(count, 5);
    assert.equal(canReleaseChatTurn(turn), false, 'obsolete aggregate has no authority');
    assert.deepEqual(timeline.messages.map(message => message.id), [identity.assistantMessageId]);
    const answer = timeline.messages.find(message => message.id === identity.assistantMessageId)!;
    assert.ok(answer.parts.every(part => part.source === 'journal'));
    assert.ok(timelineMessageText(answer).includes('临时答复'));
    assert.equal(timeline.committedSubmissions.length, 0);
    const evidence = (await terminalFixture(turn).verify(update => {
        if (update) timeline = reduceChatTimeline(timeline, { type: 'journal', owner, sourceHostId: owner.hostId, runId: update.through.identity.run_id, update }).state;
    }))!;
    timeline = reduceChatTimeline(timeline, { type: 'terminal', owner, evidence }).state;
    turn = reduceChatTurn(turn, { type: 'terminal_verified', owner, evidence }).state;
    assert.equal(canReleaseChatTurn(turn), true);
    assert.equal(timeline.committedSubmissions.length, 1);
});

test('successful transport EOF with live text stays uncertain and retains the original submission', async t => {
    Object.defineProperty(globalThis, 'document', { configurable: true, value: { cookie: '' } });
    t.after(() => Reflect.deleteProperty(globalThis, 'document'));
    t.mock.method(globalThis, 'fetch', async () => new Response(`data:${JSON.stringify(live)}\n\ndata:[DONE]\n\n`, { headers: { 'Content-Type': 'text/event-stream' } }));
    let turn = createChatTurn(owner, identity);
    let pending = bindSourceDigest(createSourceSubmission(source, request), { source_host_id: owner.hostId,
        result: { identity: source, request_digest: 'a'.repeat(64) } });
    for await (const payload of chatEventStream('/api/chat/agent/v2/stream', { method: 'POST', body: JSON.stringify(sourceSubmissionBody(pending)) }))
        turn = reduceChatTurn(turn, { type: 'event', owner, payload }).state;
    turn = reduceChatTurn(turn, { type: 'transport_interrupted', owner, reason: 'eof' }).state;
    assert.equal(turn.outcome, 'uncertain');
    assert.equal(canReleaseChatTurn(turn), false);
    assert.deepEqual(turn.identity, identity);
});
