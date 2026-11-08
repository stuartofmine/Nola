import test from 'node:test';
import assert from 'node:assert/strict';
import { createConversationSubmissionController, observedGoalResumeRoute } from './conversationSubmissionController.ts';
import type { ConversationSubmissionSnapshot } from './conversationSubmissionController.ts';
import { createChatTurn } from './chatTurn.ts';
import { terminalFixture } from './assistantTerminal.fixture.ts';
import { SourceSubmissionStorage, createSourceSubmission, bindSourceDigest, acceptSourceReceipt } from './sourceSubmission.ts';

for (const validManifest of [true, false]) for (const draftFailure of ['none', 'missing', 'retire']) test(`history read failure cannot override Core terminal authority (valid manifest: ${validManifest}, draft failure: ${draftFailure})`, async () => {
    const missingDraft = draftFailure === 'missing';
    const data = new Map<string, string>();
    const memory = { get length() { return data.size; }, key: (index: number) => [...data.keys()][index] ?? null,
        getItem: (key: string) => data.get(key) ?? null, setItem: (key: string, value: string) => { data.set(key, value); },
        removeItem: (key: string) => { data.delete(key); } };
    const storage = new SourceSubmissionStorage(memory);
    const owner = {hostId: 'host', clientId: 'client', conversationId: 'conversation', generation: 1};
    const turn = createChatTurn(owner, 'submission');
    const f = terminalFixture(turn);
    const identity = {source_host_id: owner.hostId, client_id: owner.clientId, conversation_id: owner.conversationId,
        submission_id: 'submission', user_message_id: turn.identity.userMessageId, assistant_message_id: turn.identity.assistantMessageId};
    const request = {conversationId: owner.conversationId, expected_host_id: owner.hostId, client_id: owner.clientId,
        submission_id: 'submission', user_message_id: identity.user_message_id, assistant_message_id: identity.assistant_message_id};
    const receipt = {protocol_version: 1, identity, request_digest: 'a'.repeat(64), phase: 'admitted', admission: f.seal.admission, rejection_code: null};
    const pending = acceptSourceReceipt(bindSourceDigest(createSourceSubmission(identity, request),
        {source_host_id: 'host', result: {identity, request_digest: receipt.request_digest}}), receipt);
    storage.save(pending);
    let snapshot: ConversationSubmissionSnapshot | undefined;
    let manifestRead!: () => void;
    const read = new Promise<void>(resolve => { manifestRead = resolve; });
    const workspace = {
        load: async () => {}, isLoaded: () => true, restoreSubmissions: async () => [],
        updateSubmission: async () => { if (missingDraft) throw Error('Missing durable draft ticket'); }, retireSubmission: async () => { assert.equal(missingDraft, false); if (draftFailure === 'retire') throw Error('Draft storage unavailable'); },
        submissions: new Map(), replyNotices: new Map(), knowledge: new Map(),
        text: {read: () => ''}, attachments: {read: () => []}, completions: {watch: async () => {}},
    };
    const controller = createConversationSubmissionController({
        conversation: {conversationId: 'conversation'} as never, hostId: 'host', clientId: 'client', storage,
        workspace: workspace as never, recoveryOwners: async () => [],
        request: (async (path: string, options?: RequestInit) => {
            if (path.includes('/queued-input')) return {head: null, dispatch: null};
            if (!path.includes('/agent/v2/')) throw new Error('History unavailable');
            const body = JSON.parse(String(options?.body));
            if (path.endsWith('/observe')) return {source_host_id: 'host', conversation_id: 'conversation', client_id: 'client', goal_control: null, result: {
                protocol_version: 1, conversation_id: 'conversation', client_id: 'client', run_id: 'submission',
                assistant_message_id: turn.identity.assistantMessageId, status: 'sealed', worker: {state: 'absent', generation: null},
                recovery: 'terminal', admission: f.seal.admission, terminal_seal: f.seal, permissions: [], can_submit: true,
                submission_receipt: receipt, goal_execution: null, context_usage: null,
            }};
            if (body.operation === 'heads') return {source_host_id: 'host', result: [f.through]};
            const response = await f.transport(body);
            if (body.operation === 'terminal_manifest') {
                manifestRead();
                if (!validManifest) return {source_host_id: 'host', result: null};
            }
            return response;
        }) as never,
    });
    try {
        controller.attach(value => { snapshot = value; });
        await controller.initialize();
        await Promise.race([read, new Promise((_, reject) => setTimeout(() => reject(new Error('No terminal read')), 2000))]);
        await new Promise(resolve => setTimeout(resolve, 25));
        assert.equal(snapshot?.turn?.phase === 'committed', validManifest);
        assert.equal(snapshot?.observedCanSubmit, validManifest);
        if (draftFailure !== 'none') assert.equal(snapshot?.notice, 'chatPage.draftStorageFailed');
        if (validManifest) {
            assert.equal(snapshot?.timeline?.submissionParentMessageId, turn.identity.assistantMessageId);
            assert.equal(storage.read('host', 'client', 'conversation'), null);
        }
    } finally {
        controller.dispose();
    }
});

test('recovery of a running Goal resumes its Run rather than issuing invalid Goal Resume', () => {
    const owner = {goal_id:'goal',contract_id:'contract',run_id:'run',control_token:'opaque'};
    const control = {goal_id:'goal',control_token:'opaque',status:'running',can_submit:false,can_pause:true,can_resume:false,can_cancel:true};
    assert.equal(observedGoalResumeRoute(owner,control),'run');
    for(const status of ['paused','needs_user','failed']) assert.equal(observedGoalResumeRoute(owner,{...control,status,can_resume:true}),'goal');
    for(const status of ['completed','cancelled','unknown']) assert.equal(observedGoalResumeRoute(owner,{...control,status}),'refresh');
    assert.equal(observedGoalResumeRoute(owner,{...control,status:'needs_user',can_resume:false}),'refresh');
    assert.equal(observedGoalResumeRoute(owner,null),'refresh');
    assert.equal(observedGoalResumeRoute(owner,{...control,control_token:'changed'}),'refresh');
    assert.equal(observedGoalResumeRoute(owner,{...control,goal_id:'other'}),'refresh');
    assert.equal(observedGoalResumeRoute(null,null),'run');
});

test('cold unconfirmed control remains non-submittable and explicit retry performs a fresh authority read', async () => {
    const { ApiError } = await import('./api.ts');
    const data = new Map<string,string>();
    const storage = new SourceSubmissionStorage({ get length(){return data.size;},key:i=>[...data.keys()][i]??null,
        getItem:k=>data.get(k)??null,setItem:(k,v)=>{data.set(k,v);},removeItem:k=>{data.delete(k);} });
    const workspace = {load:async()=>{},isLoaded:()=>true,restoreSubmissions:async()=>[],updateSubmission:async()=>{},retireSubmission:async()=>{},
        submissions:new Map(),replyNotices:new Map(),knowledge:new Map(),text:{read:()=>''},attachments:{read:()=>[]},completions:{watch:async()=>{}}};
    let verified = false; let reads = 0; let snapshot: ConversationSubmissionSnapshot | undefined;
    const controller = createConversationSubmissionController({conversation:{conversationId:'conversation'} as never,
        hostId:'host',clientId:'client',storage,workspace:workspace as never,recoveryOwners:async()=>[],
        request:(async(path:string)=>{if(path.endsWith('/observe')){reads++;if(!verified)throw new ApiError(409,'unconfirmed','run_control_unconfirmed');return {source_host_id:'host',conversation_id:'conversation',client_id:'client',goal_control:null,result:null};}
            if(path.includes('/queued-input'))return {head:null,dispatch:null};throw Error('History unavailable');}) as never});
    try {
        controller.attach(value=>{snapshot=value;});await controller.initialize();
        for(let i=0;i<50&&snapshot?.error!=='chatPage.controlUnconfirmed';i++)await new Promise(r=>setTimeout(r,10));
        assert.equal(reads,1);assert.equal(snapshot?.observedCanSubmit,false);assert.equal(snapshot?.challenge,null);
        assert.equal(snapshot?.observedRecovery,'control_unconfirmed');
        verified=true;controller.observe();
        for(let i=0;i<50&&!snapshot?.observedCanSubmit;i++)await new Promise(r=>setTimeout(r,10));
        assert.equal(reads,2);assert.equal(snapshot?.observedRecovery,'observe');assert.notEqual(snapshot?.error,'chatPage.controlUnconfirmed');
        // History is deliberately unavailable here: authority retry alone must not unlock the composer.
        assert.equal(snapshot?.observedCanSubmit,false);
    } finally {controller.dispose();}
});

test('active child reads journals and seals without browser control, resume, stop or new submission', async () => {
    const owner={hostId:'host',clientId:'client',conversationId:'conversation',generation:1};
    const turn=createChatTurn(owner,'child-run');
    const f=terminalFixture(turn, 'completed', [], true);
    const data=new Map<string,string>();
    const storage=new SourceSubmissionStorage({get length(){return data.size;},key:i=>[...data.keys()][i]??null,getItem:k=>data.get(k)??null,setItem:(k,v)=>{data.set(k,v);},removeItem:k=>{data.delete(k);}});
    let retiredDrafts=0;
    const workspace={load:async()=>{},isLoaded:()=>true,restoreSubmissions:async()=>[],updateSubmission:async()=>{},retireSubmission:async()=>{retiredDrafts++;},
        submissions:new Map(),replyNotices:new Map(),knowledge:new Map(),text:{read:()=>''},attachments:{read:()=>[]},completions:{watch:async()=>assert.fail('read-only child cannot acquire browser completion control')}};
    let sealed=false;let reads=0;let snapshot:ConversationSubmissionSnapshot|undefined;
    const paths:string[]=[];
    const controller=createConversationSubmissionController({conversation:{conversationId:'conversation'} as never,hostId:'host',clientId:'client',storage,
        workspace:workspace as never,recoveryOwners:async()=>[],request:(async(path:string,options?:RequestInit)=>{
            paths.push(path);
            if(path.endsWith('/observe')){reads++;return {source_host_id:'host',conversation_id:'conversation',client_id:'client',goal_control:null,result:{
                protocol_version:1,conversation_id:'conversation',client_id:'client',run_id:'child-run',assistant_message_id:turn.identity.assistantMessageId,
                status:sealed?'sealed':'running',worker:{state:'absent',generation:null},recovery:sealed?'terminal':'observe',
                admission:f.seal.admission,terminal_seal:sealed?f.seal:null,permissions:[],can_submit:false,submission_receipt:null,goal_execution:null,context_usage:null,
                task_execution:{kind:'task_child',parent_conversation_id:'parent',parent_run_id:'parent-run',task_id:'task',child_conversation_id:'conversation'}}};}
            if(path.includes('/queued-input'))return {head:null,dispatch:null};
            if(!path.endsWith('/journal'))throw Error('History unavailable');
            const body=JSON.parse(String(options?.body));
            if(body.operation==='heads')return {source_host_id:'host',result:[f.through]};
            return f.transport(body);
        }) as never});
    const waitFor=async(check:()=>boolean)=>{for(let i=0;i<100&&!check();i++)await new Promise(resolve=>setTimeout(resolve,20));assert.ok(check());};
    try{
        controller.attach(value=>{snapshot=value;});await controller.initialize();
        await waitFor(()=>reads>0&&!!snapshot?.turn);
        assert.notEqual(snapshot?.error,'chatPage.controlUnconfirmed');assert.equal(snapshot?.observedCanSubmit,false);
        await controller.stop();await controller.resume();
        sealed=true;controller.observe();
        await waitFor(()=>snapshot?.turn?.phase==='committed');
        assert.equal(snapshot?.observedCanSubmit,false);
        assert.equal(retiredDrafts,0,'reading a child seal must not consume a submitted draft');
        assert.ok(!paths.some(path=>/\/(cancel|resume|stream)$/.test(path)));
    }finally{controller.dispose();}
});

for (const outcome of ['success', 'history-in-flight', 'wrong-host', 'unavailable', 'detached'] as const) test(`terminal witness is independent, owner scoped metadata (${outcome})`, async () => {
    const data=new Map<string,string>();
    const storage=new SourceSubmissionStorage({get length(){return data.size;},key:i=>[...data.keys()][i]??null,
        getItem:k=>data.get(k)??null,setItem:(k,v)=>{data.set(k,v);},removeItem:k=>{data.delete(k);}});
    const owner={hostId:'host',clientId:'client',conversationId:'conversation',generation:1};
    const turn=createChatTurn(owner,'witness-run');
    const f=terminalFixture(turn,'completed',[],true,'exact user text');
    const identity={source_host_id:'host',client_id:'client',conversation_id:'conversation',submission_id:'witness-run',
        user_message_id:turn.identity.userMessageId,assistant_message_id:turn.identity.assistantMessageId};
    const request={conversationId:'conversation',expected_host_id:'host',client_id:'client',submission_id:'witness-run',
        user_message_id:identity.user_message_id,assistant_message_id:identity.assistant_message_id};
    const receipt={protocol_version:1,identity,request_digest:'a'.repeat(64),phase:'admitted',admission:f.seal.admission,rejection_code:null};
    storage.save(acceptSourceReceipt(bindSourceDigest(createSourceSubmission(identity,request),{source_host_id:'host',result:{identity,request_digest:receipt.request_digest}}),receipt));
    let snapshot:ConversationSubmissionSnapshot|undefined,historyReads=0,captures=0,restores=0;
    let beforeHydration:ConversationSubmissionSnapshot['timeline']=null;
    let rejectInitial!:(cause:unknown)=>void;
    const initialResponse=new Promise<unknown>((_resolve,reject)=>{rejectInitial=reject;});
    // Only this case consumes the pending initial read.
    void initialResponse.catch(()=>{});
    const success=outcome==='success'||outcome==='history-in-flight';
    let complete!:(value:unknown)=>void;
    const response=new Promise<unknown>(resolve=>{complete=resolve;});
    const paths:string[]=[];
    const workspace={load:async()=>{},isLoaded:()=>true,restoreSubmissions:async()=>[],updateSubmission:async()=>{},retireSubmission:async()=>{},
        submissions:new Map(),replyNotices:new Map(),knowledge:new Map(),text:{read:()=> 'new draft must survive'},attachments:{read:()=>[]},completions:{watch:async()=>{}}};
    const controller=createConversationSubmissionController({conversation:{conversationId:'conversation'} as never,
        hostId:'host',clientId:'client',storage,workspace:workspace as never,recoveryOwners:async()=>[],
        request:(async(path:string,options?:RequestInit)=>{
            paths.push(path);
            if(path.includes('/queued-input'))return {head:null,dispatch:null};
            if(path.includes('/messages')){historyReads++;if(historyReads===1){if(outcome==='history-in-flight')return initialResponse;throw Error('Initial history unavailable');}return response;}
            if(path.endsWith('/observe'))return {source_host_id:'host',conversation_id:'conversation',client_id:'client',goal_control:null,result:{
                protocol_version:1,conversation_id:'conversation',client_id:'client',run_id:'witness-run',assistant_message_id:turn.identity.assistantMessageId,
                status:'sealed',worker:{state:'absent',generation:null},recovery:'terminal',admission:f.seal.admission,terminal_seal:f.seal,
                permissions:[],can_submit:true,submission_receipt:receipt,goal_execution:null,context_usage:null}};
            const body=JSON.parse(String(options?.body));
            if(body.operation==='heads')return {source_host_id:'host',result:[f.through]};
            return f.transport(body);
        }) as never});
    const wait=async(check:()=>boolean)=>{for(let i=0;i<150&&!check();i++)await new Promise(resolve=>setTimeout(resolve,10));assert.ok(check());};
    try{
        controller.attach(value=>{snapshot=value;},{captureHistory:reason=>{assert.equal(reason,'metadata');beforeHydration=snapshot!.timeline;captures++;return {anchor:'retained'};},
            restoreHistory:async()=>{restores++;}});
        const initialization=controller.initialize();
        if(outcome==='history-in-flight'){
            await wait(()=>historyReads===1);
            controller.observe(); // Public authority refresh while the initial transcript request is pending.
            await wait(()=>snapshot?.turn?.phase==='committed');
            assert.equal(historyReads,1,'Metadata must not invalidate the in-flight initial history');
            rejectInitial(Error('Initial history unavailable'));
        }
        await initialization;
        await wait(()=>historyReads===2&&snapshot?.turn?.phase==='committed');
        const before=snapshot!.timeline!;
        assert.equal(before.messages.find(m=>m.role==='user')?.canonical,null);
        if(outcome==='detached')controller.detach();
        const payload={sourceHostId:outcome==='wrong-host'?'foreign':'host',conversationProjection:{contract_version:1,conversation_id:'conversation',revision:37,
            messages:[{message_id:turn.identity.userMessageId,role:'user',state:'completed',revision:4,created_at_ms:1000,
                parts:[{kind:'text',part_id:`${turn.identity.userMessageId}:body`,revision:2,markdown:'exact user text',is_final:true,review:'approved'}]}]},
            messageWindow:{projectionRevision:37,eventSequence:99,before:null,hasMore:false,totalCount:2}};
        complete(outcome==='unavailable'?null:payload);
        if(success)await wait(()=>!!snapshot?.timeline?.messages.find(m=>m.role==='user')?.canonical);
        else await new Promise(resolve=>setTimeout(resolve,30));
        const after=snapshot!.timeline!,user=after.messages.find(m=>m.role==='user')!;
        // A public authority refresh may legitimately republish admissions while
        // the initial history settles. Compare against the boundary captured
        // immediately before metadata application, not before that refresh.
        const preserved=beforeHydration??before;
        assert.equal(snapshot!.turn!.phase,'committed');
        assert.equal(snapshot!.draft,'new draft must survive');
        assert.equal(user.canonical?.revision??null,success?4:null);
        assert.strictEqual(user.parts,preserved.messages.find(m=>m.role==='user')!.parts);
        assert.strictEqual(after.window,before.window);
        assert.strictEqual(after.projection,before.projection);
        assert.strictEqual(after.journalCursors,before.journalCursors);
        assert.equal(after.submissionParentMessageId,before.submissionParentMessageId);
        assert.equal(captures,success?1:0);assert.equal(restores,captures);
        assert.equal(historyReads,2,'Exactly initial history and one independent metadata read');
        assert.ok(paths.every(path=>!path.endsWith('/submit')&&!path.endsWith('/resume')&&!path.endsWith('/control')));
    }finally{controller.dispose();}
});
