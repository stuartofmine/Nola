import test from 'node:test';
import assert from 'node:assert/strict';
import { createChatTimeline, timelineRequest, reduceChatTimeline, parseCanonicalProjection, timelineMessageText, timelineMessageReasoning, timelinePartText, timelineToolPresentation, timelineReasoningParts, timelineShowsWorkingPlaceholder } from './chatTimeline.ts';
import type { ChatTimeline, TimelineAction, CanonicalMessage, CanonicalProjection } from './chatTimeline.ts';
import { journalPart, journalUpdate } from './journalTimeline.fixture.ts';
import type { JournalUpdate } from './assistantJournal.ts';
import {terminalFixture} from './assistantTerminal.fixture.ts';
import {createChatTurn} from './chatTurn.ts';

const owner = { conversationId: 'session-a', generation: 1 };
const identity = { conversation_id: owner.conversationId, run_id: 'run-a', assistant_message_id: 'assistant-a', source_store_epoch: 'epoch' };
const message = (id: string, role: CanonicalMessage['role'] = 'assistant', text = id): CanonicalMessage => ({ message_id: id, role, state: 'completed', revision: 1, created_at_ms: 1, parts: [{ kind: 'text', part_id: `${id}:text`, revision: 1, markdown: text, is_final: true }] });
const projection = (messages: CanonicalMessage[], revision = 1): CanonicalProjection => ({ contract_version: 1, conversation_id: owner.conversationId, revision, messages, surface: { revision, committed_message_ids: messages.map(item => item.message_id), live_message_id: null } });
const history = (messages: CanonicalMessage[], revision = 1) => ({ sourceHostId:'host', conversationProjection: projection(messages, revision), messageWindow: { projectionRevision: revision, eventSequence: 90, before: 4, hasMore: true, totalCount: 10 } });
const update = (sequence: string, part_id = 'p', markdown = '你好') => journalUpdate(sequence, identity, [{ kind: 'text', part_id, markdown }]);
const apply = (state: ChatTimeline, value: JournalUpdate) => reduceChatTimeline(state, { type: 'journal', owner, sourceHostId: 'host', runId: value.through.identity.run_id, update: value });

async function terminalWitnessFixture() {
    const turn=createChatTurn({...owner,hostId:'host',clientId:'client'},'witness-submission');
    const protocol=terminalFixture(turn,'completed',[],true,'Exact public question');
    const initial=history(Array.from({length:64},(_,index)=>message(`old-${index}`)),4);
    initial.messageWindow.totalCount=80;
    let state=reduceChatTimeline(createChatTimeline(owner),{type:'history',request:timelineRequest(createChatTimeline(owner)),mode:'replace',payload:initial}).state;
    state=reduceChatTimeline(state,{type:'beginSubmission',owner,submissionId:turn.identity.submissionId,
        userMessageId:turn.identity.userMessageId,userText:'Exact public question'}).state;
    const evidence=await protocol.verify(value=>{if(value)state=apply(state,value).state;});
    assert.ok(evidence);
    state=reduceChatTimeline(state,{type:'terminal',owner,evidence}).state;
    const canonical=message(turn.identity.userMessageId,'user','Exact public question');
    canonical.parts[0].part_id=`${canonical.message_id}:body`;
    canonical.revision=3;canonical.parts[0].revision=3;
    const payload=history([canonical],9);payload.messageWindow.eventSequence=95;
    return {state,evidence,payload,userId:canonical.message_id};
}

test('history installs canonical snapshot and rejects a missing source Host instead of inventing one',()=>{
    const initial=createChatTimeline(owner),payload=history([message('user','user')],5);
    const admitted=reduceChatTimeline(initial,{type:'history',request:timelineRequest(initial),mode:'replace',payload});
    assert.deepEqual(admitted.state.messages[0].canonicalSnapshot,{sourceHostId:'host',conversationId:owner.conversationId,revision:5});
    assert.ok(Object.isFrozen(admitted.state.messages[0].canonicalSnapshot));
    const missing={...payload} as Record<string,unknown>;delete missing.sourceHostId;
    assert.equal(reduceChatTimeline(initial,{type:'history',request:timelineRequest(initial),mode:'replace',payload:missing}).ignored,'invalid');
});

test('verified terminal hydrates only its exact user witness without replacing the loaded range or journal',async()=>{
    const {state,evidence,payload,userId}=await terminalWitnessFixture();
    const before=state.messages.find(message=>message.id===userId)!;
    assert.equal(before.canonical,null);
    const result=reduceChatTimeline(state,{type:'terminal_witness',request:timelineRequest(state),sourceHostId:'host',evidence,payload});
    assert.equal(result.ignored,undefined);
    assert.equal(result.state.version,state.version+1);
    assert.equal(result.state.messages.length,66);
    assert.deepEqual(result.state.messages.map(message=>message.id),state.messages.map(message=>message.id));
    const after=result.state.messages.find(message=>message.id===userId)!;
    assert.equal(after.parts,before.parts);
    assert.deepEqual(after.canonical,payload.conversationProjection.messages[0]);
    assert.deepEqual(after.canonicalSnapshot,{sourceHostId:'host',conversationId:owner.conversationId,revision:9});
    assert.equal(before.canonical,null,'The old timeline remains unchanged');
    assert.equal(result.state.projection,state.projection);
    assert.equal(result.state.window,state.window);
    assert.equal(result.state.journalCursors,state.journalCursors);
    assert.equal(result.state.committedSubmissions,state.committedSubmissions);
    assert.equal(result.state.submissionParentMessageId,state.submissionParentMessageId);
    assert.equal(result.state.durable,true);
    for(const row of state.messages.filter(message=>message.id!==userId))assert.equal(result.state.messages.find(message=>message.id===row.id),row);
    payload.conversationProjection.messages[0].parts[0].markdown='Mutated transport';
    assert.equal(after.canonical?.parts[0].markdown,'Exact public question');
});

test('repeated exact admissions keep the hydrated user Parts and snapshot references while optimistic Parts are replaced',async()=>{
    const {state,evidence,payload,userId}=await terminalWitnessFixture();
    let hydrated=reduceChatTimeline(state,{type:'terminal_witness',request:timelineRequest(state),sourceHostId:'host',evidence,payload}).state;
    const original=hydrated.messages.find(message=>message.id===userId)!;
    assert.ok(original.canonicalSnapshot);
    for(let repeat=0;repeat<2;repeat++){
        hydrated=reduceChatTimeline(hydrated,{type:'admissions',owner,messages:evidence.declarations}).state;
        const next=hydrated.messages.find(message=>message.id===userId)!;
        assert.strictEqual(next.parts,original.parts);
        assert.strictEqual(next.canonical,original.canonical);
        assert.strictEqual(next.canonicalSnapshot,original.canonicalSnapshot);
    }
    const optimisticParts=[{source:'optimistic' as const,id:original.parts[0].id,value:{kind:'text' as const,markdown:'Exact public question'}}];
    const optimistic={...hydrated,messages:hydrated.messages.map(message=>message.id===userId?{...message,parts:optimisticParts}:message)};
    const admitted=reduceChatTimeline(optimistic,{type:'admissions',owner,messages:evidence.declarations}).state.messages.find(message=>message.id===userId)!;
    assert.notStrictEqual(admitted.parts,optimisticParts);
    assert.equal(admitted.parts[0].source,'canonical');
    assert.strictEqual(admitted.canonicalSnapshot,original.canonicalSnapshot);
});

test('missing-snapshot hydration cannot lower a known revision of the same canonical Part',async()=>{
    const {state,evidence,payload,userId}=await terminalWitnessFixture();
    const previous=structuredClone(payload.conversationProjection.messages[0]);
    previous.parts[0].revision++;
    const partial={...state,messages:state.messages.map(message=>message.id===userId?{...message,canonical:previous,canonicalSnapshot:null}:message)};
    const result=reduceChatTimeline(partial,{type:'terminal_witness',request:timelineRequest(partial),sourceHostId:'host',evidence,payload});
    assert.equal(result.ignored,'invalid');
    assert.strictEqual(result.state,partial);
    const current=structuredClone(payload);current.conversationProjection.messages[0].parts[0].revision=previous.parts[0].revision;
    const accepted=reduceChatTimeline(partial,{type:'terminal_witness',request:timelineRequest(partial),sourceHostId:'host',evidence,payload:current});
    assert.equal(accepted.ignored,undefined);
    assert.ok(accepted.state.messages.find(message=>message.id===userId)?.canonicalSnapshot);
});

test('terminal witness rejects stale owner/request/Run and foreign or regressing read snapshots',async()=>{
    const {state,evidence,payload}=await terminalWitnessFixture();
    const action:Extract<TimelineAction,{type:'terminal_witness'}>={type:'terminal_witness',request:timelineRequest(state),sourceHostId:'host',evidence,payload};
    assert.equal(reduceChatTimeline(state,{...action,request:{...action.request,owner:{...owner,generation:owner.generation+2}}}).ignored,'owner');
    assert.equal(reduceChatTimeline(state,{...action,request:{...action.request,version:state.version-1}}).ignored,'stale_request');
    assert.equal(reduceChatTimeline({...state,durable:false},action).ignored,'submission');
    assert.equal(reduceChatTimeline({...state,activeSubmissionId:'new-submission'},action).ignored,'submission');
    assert.equal(reduceChatTimeline({...state,committedSubmissions:[]},action).ignored,'submission');
    assert.equal(reduceChatTimeline(state,{...action,evidence:structuredClone(evidence)}).ignored,'owner','Unverified evidence cannot grant a witness');
    for(const mutate of [
        (value:typeof payload)=>{value.sourceHostId='foreign';},
        (value:typeof payload)=>{value.conversationProjection.conversation_id='foreign';},
        (value:typeof payload)=>{value.messageWindow.projectionRevision++;},
        (value:typeof payload)=>{value.messageWindow.eventSequence=1;},
        (value:typeof payload)=>{value.conversationProjection.revision=3;value.messageWindow.projectionRevision=3;},
        (value:typeof payload)=>{value.conversationProjection.messages[0].parts[0].markdown='Changed body';},
        (value:typeof payload)=>{value.conversationProjection.messages[0].parts[0].part_id='different-part';},
        (value:typeof payload)=>{value.conversationProjection.messages[0].parts[0].review='pending';},
        (value:typeof payload)=>{value.conversationProjection.messages[0].role='assistant';},
        (value:typeof payload)=>{value.conversationProjection.messages[0].revision=0;},
    ]){
        const changed=structuredClone(payload);mutate(changed);
        const result=reduceChatTimeline(state,{...action,payload:changed});
        assert.ok(result.ignored);
        assert.equal(result.state,state);
    }
});

test('cold committed terminal is eligible but absent/unrelated users and existing witnesses are never manufactured',async()=>{
    const {state,evidence,payload,userId}=await terminalWitnessFixture();
    const cold={...state,activeSubmissionId:null};
    const applied=reduceChatTimeline(cold,{type:'terminal_witness',request:timelineRequest(cold),sourceHostId:'host',evidence,payload}).state;
    assert.ok(applied.messages.find(message=>message.id===userId)?.canonicalSnapshot);
    const again=reduceChatTimeline(applied,{type:'terminal_witness',request:timelineRequest(applied),sourceHostId:'host',evidence,payload});
    assert.equal(again.state,applied,'Existing paired witness is not replaced');
    const unrelated=history([message('old-0','user','Unrelated')],9);unrelated.messageWindow.eventSequence=95;
    assert.equal(reduceChatTimeline(state,{type:'terminal_witness',request:timelineRequest(state),sourceHostId:'host',evidence,payload:unrelated}).state,state);
    assert.equal(reduceChatTimeline(state,{type:'terminal_witness',request:timelineRequest(state),sourceHostId:'host',evidence,
        payload:{...payload,conversationProjection:{...payload.conversationProjection,messages:[]}}}).state,state);
});

test('working placeholder has one live owner and never duplicates content or run status', () => {
    const row = {id: 'live', role: 'assistant' as const, state: 'running' as const, canonical: null, parts: []};
    assert.equal(timelineShowsWorkingPlaceholder(row, 'live', false), true);
    assert.equal(timelineShowsWorkingPlaceholder(row, 'live', true), false);
    assert.equal(timelineShowsWorkingPlaceholder(row, 'other', false), false);
    assert.equal(timelineShowsWorkingPlaceholder(row, null, false), false);
    assert.equal(timelineShowsWorkingPlaceholder({...row, state: 'completed'}, 'live', false), false);
    assert.equal(timelineShowsWorkingPlaceholder({...row, role: 'user'}, 'live', false), false);
    for (const value of [
        {kind: 'text', markdown: 'Answer'}, {kind: 'reasoning', summary: 'Analysis'},
        {kind: 'tool', state: 'waiting_for_permission'}, {kind: 'error'}, {kind: 'task'},
    ]) {
        const parts = [{source: 'canonical' as const, id: 'part', value: {part_id: 'part', revision: 1, ...value}}];
        assert.equal(timelineShowsWorkingPlaceholder({...row, parts}, 'live', false), false);
    }
});

test('empty reasoning placeholders do not become history disclosures, but content and failures survive', () => {
    const canonical = message('assistant');
    canonical.parts = [
        {kind: 'reasoning', part_id: 'empty', revision: 1, state: 'completed', title: '正在思考中', summary: '  '},
        {kind: 'reasoning', part_id: 'active', revision: 1, state: 'running', title: 'Thinking'},
        {kind: 'reasoning', part_id: 'body', revision: 1, state: 'completed', summary: '实际分析'},
        {kind: 'reasoning', part_id: 'failure', revision: 1, state: 'failed', title: 'Failure'},
    ];
    const displayed = {id: canonical.message_id, role: canonical.role, state: canonical.state, canonical,
        parts: canonical.parts.map(value => ({source: 'canonical' as const, id: value.part_id, value}))};
    assert.deepEqual(timelineReasoningParts(displayed).map(part => part.id), ['body', 'failure']);
    assert.equal(displayed.parts.length, 4, 'display policy must not mutate canonical history');
});

test('tool display retains independent fields and distinguishes missing, null and empty results', () => {
    for (const result of [{ present: false }, { present: true, value: null }, { present: true, value: '' }, { present: true, value: [] }, { present: true, value: {} }]) {
        const value = {kind: 'tool', execution_source: {kind: 'model'}, part_id: 'part', revision: 77, title: ' 独立标题 ', summary: '', output_preview: '独立预览', ...(result.present ? {output: result.value} : {})};
        for (const part of [{source: 'canonical' as const, id: 'part', value}, journalPart(identity, value)]) {
            const presentation = timelineToolPresentation(part)!;
            assert.equal(presentation.title, ' 独立标题 ');
            assert.equal(presentation.summary, '');
            assert.equal(presentation.outputPreview, '独立预览');
            assert.equal(presentation.hasStructuredResult, result.present);
            assert.equal(Object.hasOwn(presentation, 'structuredResult'), result.present);
            if (result.present) assert.deepEqual(presentation.structuredResult, result.value);
        }
    }
});
test('review states cannot disclose drafts through display selectors', () => {
    for (const review of [null, 'pending', 'rejected', 'interrupted', 'approved']) {
        const part = journalPart(identity, {kind:'text', part_id:'p', review, markdown:' draft '});
        assert.equal(timelinePartText(part), review === null || review === 'approved' ? ' draft ' : '');
    }
});
test('canonical descriptors preserve exact ownership and citation absence without accepting authority', () => {
    const canonical = message('message');
    const tool = {kind: 'tool', execution_source: {kind: 'model'}, part_id: 'tool', revision: 9, tool_call_id: 'call',
        full_output: {message_id: 'message', part_id: 'tool', tool_call_id: 'call'}};
    canonical.parts.push(tool, {kind: 'citation', part_id: 'absent', revision: 1, citation_id: 'real-absent', location: null},
        {kind: 'citation', part_id: 'empty', revision: 2, citation_id: 'real-empty', location: {}},
        {kind: 'citation', part_id: 'section', revision: 3, citation_id: 'real-section', location: {section: ' 章节 '}, section: ' 章节 '});
    const parsed = parseCanonicalProjection(projection([canonical]));
    assert.equal(parsed.messages[0].parts[2].location, null);
    assert.deepEqual(parsed.messages[0].parts[3].location, {});
    assert.equal(parsed.messages[0].parts[4].citation_id, 'real-section');
    for (const field of ['message_id', 'part_id', 'tool_call_id', 'token', 'grant', 'path']) {
        const modified = structuredClone(canonical);
        modified.parts[1].full_output = {...tool.full_output, [field]: 'invalid'};
        assert.throws(() => parseCanonicalProjection(projection([modified])));
    }
    const conflicting = structuredClone(canonical);
    conflicting.parts[4].section = 'conflicting';
    assert.throws(() => parseCanonicalProjection(projection([conflicting])));
});


test('public tool descriptors never accept private paths or incomplete arguments', () => {
    for (const field of ['raw_input', 'full_output_ref']) {
        const row = message('a');
        row.parts.push({kind:'tool', part_id:'tool', revision:1, [field]:'SECRET'});
        assert.throws(() => parseCanonicalProjection(projection([row])));
    }
});
test('canonical history preserves exact order, unknown part fields, and input immutability', () => {
    const state = createChatTimeline(owner);
    const payload = history([message('u', 'user'), message('a')]);
    payload.conversationProjection.messages[1].parts.push({ kind: 'future_kind', part_id: 'opaque', revision: 1, payload: { x: 'never infer tool permissions' } });
    const result = reduceChatTimeline(state, { type: 'history', request: timelineRequest(state), mode: 'replace', payload });
    assert.equal(result.ignored, undefined);
    assert.deepEqual(result.state.messages.map(item => item.id), ['u', 'a']);
    payload.conversationProjection.messages[1].parts[0].markdown = 'changed';
    assert.equal(result.state.messages[1].parts[0].value.markdown, 'a');
    assert.equal(state.messages.length, 0);
});

test('journal replacement preserves stable ordering, whitespace and lossless replay clocks', () => {
    let state = apply(createChatTimeline(owner), update('9007199254740993', 'text', ' a ')).state;
    state = apply(state, journalUpdate('9007199254740994', identity, [
        {kind:'reasoning', part_id:'thinking', summary:'思考'}
    ])).state;
    state = apply(state, update('9007199254740995', 'text', ' a b ')).state;
    assert.deepEqual(state.messages[0].parts.map(part => part.id), ['text','thinking']);
    assert.equal(timelineMessageText(state.messages[0]), ' a b ');
    assert.equal(timelineMessageReasoning(state.messages[0]), '思考');
    assert.equal(apply(state, update('9007199254740995')).ignored, 'duplicate');
    assert.equal(apply(state, update('9007199254740994')).ignored, 'duplicate');
    const forged = update('9007199254740995'); forged.through.record_digest = 'f'.repeat(64);
    assert.equal(apply(state, forged).ignored, 'invalid');
});
test('conversation, generation, source and Run reject stale or foreign callbacks', () => {
    assert.equal(apply(createChatTimeline({...owner,generation:3}), update('1')).ignored,'owner');
    let state = apply(createChatTimeline(owner),update('1')).state;
    const foreign = update('2'); foreign.through.identity = {...identity,conversation_id:'other'};
    assert.equal(apply(state,foreign).ignored,'owner');
    assert.equal(reduceChatTimeline(state,{type:'journal',owner,sourceHostId:'other',runId:'run-a',update:update('2')}).ignored,'owner');
    assert.equal(reduceChatTimeline(state,{type:'journal',owner,sourceHostId:'host',runId:'other',update:update('2')}).ignored,'owner');
    assert.equal(reduceChatTimeline(state,{type:'history',request:timelineRequest(createChatTimeline({...owner,generation:0})),mode:'replace',payload:history([])}).ignored,'owner');
});
test('late history cannot clobber journal changes; prepend pins both snapshot clocks', () => {
    let state = createChatTimeline(owner); const before = timelineRequest(state);
    state = apply(state,update('1')).state;
    assert.equal(reduceChatTimeline(state,{type:'history',request:before,mode:'replace',payload:history([])}).ignored,'stale_request');
    state = reduceChatTimeline(state,{type:'history',request:timelineRequest(state),mode:'replace',payload:history([message('middle'),message('last')])}).state;
    const page=history([message('first'),message('middle')]);
    const added=reduceChatTimeline(state,{type:'history',request:timelineRequest(state),mode:'prepend',payload:page}).state;
    assert.deepEqual(added.messages.map(row=>row.id),['first','middle','last']);
    assert.deepEqual(added.projection!.messages.map(row=>row.message_id),['first','middle','last']);
    page.messageWindow.eventSequence++;
    assert.equal(reduceChatTimeline(state,{type:'history',request:timelineRequest(state),mode:'prepend',payload:page}).ignored,'snapshot_changed');
});
test('obsolete body envelopes, trace terminals and DONE are not transcript actions or durability', () => {
    const state=apply(createChatTimeline(owner),update('1')).state;
    for(const type of ['core_part_updated','conversation_projection_committed_v1','done','core_session_projection_updated','core_task_updated']) {
        const result=reduceChatTimeline(state,{type,owner} as unknown as TimelineAction);
        assert.equal(result.ignored,'unsupported'); assert.equal(result.state,state); assert.equal(result.state.durable,false);
    }
});
test('historical selectors honor complete fields and never reinterpret a tool as assistant prose', () => {
    const historical=(schema:string,canonical_kind:string,fields:Record<string,unknown>)=>({source:'canonical' as const,id:'h',value:{kind:'historical',part_id:'h',revision:1,projection:{schema,canonical_kind,status:{state:'supported'},fields,canonical_text:{text:'preview',total_utf8_bytes:400}}}});
    assert.equal(timelinePartText(historical('message_part_view','text',{markdown:{availability:'complete',value:'exact'}})),'exact');
    assert.equal(timelinePartText(historical('agent_part','text',{text:{availability:'deferred',value:'not complete'}})),'preview');
    assert.equal(timelinePartText(historical('agent_part','reasoning',{detail:{availability:'complete',value:'think'}}),'reasoning'),'think');
    assert.equal(timelinePartText(historical('agent_part','tool',{text:{availability:'complete',value:'not prose'}})),'');
});
test('subsequent Runs own independent journal clocks and optimistic input carries no authority', () => {
    let state=reduceChatTimeline(createChatTimeline(owner),{type:'beginSubmission',owner,submissionId:'run-a',userMessageId:'u',userText:' exact '}).state;
    assert.equal(timelineMessageText(state.messages[0]),' exact ');
    assert.equal(state.messages[0].parts[0].source,'optimistic');
    state=apply(state,update('90')).state;
    state=reduceChatTimeline(state,{type:'beginSubmission',owner,submissionId:'run-b'}).state;
    const next=journalUpdate('1',{...identity,run_id:'run-b',assistant_message_id:'assistant-b'},[{kind:'text',part_id:'b-text',markdown:'next'}]);
    state=apply(state,next).state;
    assert.deepEqual(state.messages.map(row=>row.id),['u','assistant-a','assistant-b']);
    assert.equal(state.durable,false);
});
test('stable Part IDs cannot migrate to another message, malformed cursors fail closed', () => {
    const state=apply(createChatTimeline(owner),update('1')).state;
    const moved=journalUpdate('2',{...identity,assistant_message_id:'other'},[{kind:'text',part_id:'p',markdown:'bad'}]);
    assert.equal(apply(state,moved).ignored,'invalid');
    const malformed=update('2'); malformed.through.sequence='01';
    assert.equal(apply(state,malformed).ignored,'invalid');
    assert.throws(()=>parseCanonicalProjection(projection([message('duplicate'),message('duplicate')])));
    assert.throws(()=>parseCanonicalProjection({...projection([]),contract_version:999}));
});

test('admitted steering user text replaces optimistic display without inventing completion', () => {
    let state=reduceChatTimeline(createChatTimeline(owner),{type:'beginSubmission',owner,submissionId:'run-a',userMessageId:'u',userText:'optimistic'}).state;
    state=reduceChatTimeline(state,{type:'admissions',owner,messages:[
        {ordinal:0,input:{id:'u',run_id:'run-a',role:'user',status:'completed',created_at_ms:1,completed_at_ms:1,
            initial_parts:[{id:'u:text',kind:'text',content_text:'真正输入 👋'}]}},
        {ordinal:1,input:{id:'assistant-a',run_id:'run-a',role:'assistant',status:'streaming',created_at_ms:1,completed_at_ms:null,initial_parts:[]}},
    ]}).state;
    assert.deepEqual(state.messages.map(row=>row.id),['u','assistant-a']);
    assert.equal(timelineMessageText(state.messages[0]),'真正输入 👋');
    assert.equal(state.messages[1].state,'running');
    assert.equal(state.durable,false);
});

test('retiring journal Parts removes their content without reopening a completed history row', () => {
    let state=createChatTimeline(owner);
    state=reduceChatTimeline(state,{type:'history',request:timelineRequest(state),mode:'replace',payload:history([message('assistant-a')])}).state;
    state=apply(state,update('1')).state;
    const retired=update('2'); retired.changes=[{partId:'p',part:null}];
    state=apply(state,retired).state;
    assert.equal(state.messages[0].parts.length,0);
    assert.equal(state.messages[0].state,'completed');
    assert.equal(state.durable,false);
});

test('history refresh updates submission frontier while prepending old pages preserves it', () => {
    let timeline = createChatTimeline(owner);
    timeline = reduceChatTimeline(timeline, {type: 'history', request: timelineRequest(timeline), mode: 'replace', payload: history([message('latest')])}).state;
    assert.equal(timeline.submissionParentMessageId, 'latest');
    timeline = reduceChatTimeline(timeline, {type: 'history', request: timelineRequest(timeline), mode: 'prepend', payload: history([message('older')])}).state;
    assert.equal(timeline.submissionParentMessageId, 'latest');
    timeline = reduceChatTimeline(timeline, {type: 'history', request: timelineRequest(timeline), mode: 'replace', payload: history([message('newer')], 2)}).state;
    assert.equal(timeline.submissionParentMessageId, 'newer');
});


test('canonical errors require the one current typed contract without field aliases', () => {
    const row = message('failed-owner');
    const error = {kind: 'error', part_id: 'error', revision: 1, code: 'provider_failed', display_key: 'unknown.valid.error', recoverable: false};
    row.parts.push(error);
    assert.deepEqual(parseCanonicalProjection(projection([row])).messages[0].parts[1], error);
    for (const changes of [{code: undefined}, {code: null}, {display_key: undefined}, {display_key: 1},
        {recoverable: undefined}, {recoverable: 'false'}, {failure: 'old untyped error'}]) {
        const changed = structuredClone(row); changed.parts[1] = {...error, ...changes};
        assert.throws(() => parseCanonicalProjection(projection([changed])), /Invalid canonical Error/);
    }
});
