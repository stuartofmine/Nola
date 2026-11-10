import test from 'node:test';
import assert from 'node:assert/strict';
import { journalTaskOwner, taskControl, TaskActionSubmission as BaseTaskActionSubmission, type TaskIntentLock } from './chatTaskActions.ts';
import { journalPart } from './journalTimeline.fixture.ts';

class TaskActionSubmission extends BaseTaskActionSubmission {
    constructor(...args: ConstructorParameters<typeof BaseTaskActionSubmission>) {
        super(args[0], args[1], args[2], args[3] ?? ((_name, operation) => operation()));
    }
}
const owner = {hostId: 'host', conversationId: 'conversation', runId: 'run'};
const identity = {source_store_epoch: 'epoch', conversation_id: owner.conversationId, run_id: owner.runId, assistant_message_id: 'assistant'};
const part = journalPart(identity, {kind: 'task_control', part_id: 'part', task_id: 'task', state: 'running', cancel_requested: false,
    supports_guidance: true, supports_cancellation: true});
function store() {
    const entries = new Map<string, string>();
    return {getItem: (key: string) => entries.get(key) ?? null, setItem: (key: string, value: string) => { entries.set(key, value); }, removeItem: (key: string) => { entries.delete(key); }};
}
test('task controls require exact journal ownership and explicit capabilities', () => {
    assert.ok(taskControl(part, owner));
    assert.equal(taskControl({...part, value: {...part.value, kind: 'task'}}, owner), null, 'a Task summary cannot grant TaskControl authority');
    assert.equal(taskControl(part, {...owner, runId: 'other'}), null);
    assert.deepEqual(journalTaskOwner(part,{...owner,runId:'newer-turn'} as typeof owner),owner);
    assert.equal(journalTaskOwner(part,{...owner,conversationId:'foreign'}),null);
    assert.equal(taskControl(part, {...owner, conversationId: 'other'}), null);
    assert.equal(taskControl({source: 'canonical', id: part.id, value: {...part.value, revision: 1}}, owner), null);
    for (const changes of [{state: 'future'}, {cancel_requested: undefined}]) {
        assert.equal(taskControl({...part, value: {...part.value, ...changes}}, owner), null);
    }
});
test('guidance sends exact IDs and validates a queued receipt without completing task', async () => {
    const storage = store(), submission = new TaskActionSubmission(owner, taskControl(part, owner)!, storage);
    await submission.submit('guidance', '  Check tests  ', async (path, body) => {
        assert.equal(path, '/api/chat/agent/v2/task/message');
        const intent = body as any;
        assert.equal(intent.parentRunId, 'run'); assert.equal(intent.hostId, 'host');
        assert.equal(intent.message, 'Check tests'); assert.ok(intent.messageId); assert.ok(Number.isSafeInteger(intent.submittedAtMs));
        assert.equal(new TaskActionSubmission(owner, taskControl(part, owner)!, storage).state, 'unknown');
        return receipt(body);
    });
    assert.equal(submission.state, 'queued');
    assert.equal(part.value.state, 'running');
    assert.equal(new TaskActionSubmission(owner, taskControl(part, owner)!, storage).state, 'ready');
});
test('cancellation intent survives remount and explicit recovery reconciles the exact owner', async () => {
    const storage = store(), control = taskControl(part, owner)!;
    const submission = new TaskActionSubmission(owner, control, storage);
    let calls = 0;
    const send = async () => { calls++; return {hostId:'host', conversationId:'conversation', parentRunId:'run', taskId:'task', cancelRequested:true, terminal:false, state:'running', applicationPending:false}; };
    await submission.submit('cancel', '', send);
    assert.equal(submission.state, 'cancelRequested');
    const restored = new TaskActionSubmission(owner, control, storage);
    await restored.submit('cancel', '', send);
    assert.equal(calls, 2);
    assert.equal(restored.state, 'cancelRequested');
    assert.equal(restored.canRecover, true);
    await restored.recover(send);
    assert.equal(calls, 4);
});
test('lost or foreign receipts retain uncertainty and prevent duplicate guidance on reload', async () => {
    for (const result of [null, {conversationId: 'other', taskId: 'task'}, {conversationId: 'conversation', taskId: 'task', deliveryPending: true}]) {
        const storage = store(), control = taskControl(part, owner)!;
        const submission = new TaskActionSubmission(owner, control, storage);
        let calls = 0;
        const send = async () => { calls++; return result; };
        await submission.submit('guidance', 'Instruction', send);
        assert.equal(submission.state, 'unknown');
        await new TaskActionSubmission(owner, control, storage).submit('guidance', 'Instruction', send);
        assert.equal(calls, 1);
    }
});
test('invalid input, unavailable storage, and unsupported actions never dispatch', async () => {
    let calls = 0;
    const send = async () => { calls++; return {}; };
    const control = taskControl(part, owner)!;
    const submission = new TaskActionSubmission(owner, {...control, cancellation: false}, store());
    await submission.submit('cancel', '', send);
    await submission.submit('guidance', ' ', send);
    await submission.submit('guidance', 'x'.repeat(4097), send);
    const unavailable = new TaskActionSubmission(owner, control, {...store(), setItem() { throw new Error('storage unavailable'); }});
    await unavailable.submit('guidance', 'Instruction', send);
    assert.equal(unavailable.state, 'unknown');
    assert.equal(calls, 0);
});

function receipt(body: any, status = 'accepted') {
    return {hostId:body.hostId, conversationId:body.conversationId, taskId:body.taskId, messageId:body.messageId,
        guidance:{task_id:body.taskId,message_id:body.messageId,owner_session_id:body.conversationId,
            content:body.message,submitted_at_ms:body.submittedAtMs,status}};
}
test('lost response recovers persisted original receipt after remount without resubmission', async () => {
    const storage=store(), control=taskControl(part,owner)!;
    const first=new TaskActionSubmission(owner,control,storage); let original:any;
    await first.submit('guidance','Original instruction',async(_path,body)=>{original=body;throw Error('response lost');});
    const restored=new TaskActionSubmission(owner,control,storage);
    assert.equal(restored.canRecover,true);
    const paths:string[]=[];
    await restored.recover(async(path,body)=>{paths.push(path);assert.deepEqual(body,original);return receipt(body,'delivered');});
    assert.deepEqual(paths,['/api/chat/agent/v2/task/message/receipt']);
    assert.equal(restored.state,'delivered');
});
test('authoritative missing receipt resubmits exactly the frozen intent, never a new ID or time', async()=>{
    const storage=store(), control=taskControl(part,owner)!;let original:any;
    const first=new TaskActionSubmission(owner,control,storage);
    await first.submit('guidance','Frozen',async(_p,b)=>{original=b;throw Error('offline');});
    const restored=new TaskActionSubmission(owner,control,storage);let calls=0;
    await restored.recover(async(path,body)=>{calls++;assert.deepEqual(body,original);
        return path.endsWith('/receipt')?{...receipt(body),guidance:null}:receipt(body);});
    assert.equal(calls,2);assert.equal(restored.state,'queued');
});
test('query failure or foreign receipt never retries submission',async()=>{
    for(const bad of [null,{hostId:'foreign'}]){
        const storage=store(), control=taskControl(part,owner)!;
        const first=new TaskActionSubmission(owner,control,storage);
        await first.submit('guidance','Frozen',async()=>{throw Error('offline');});
        const restored=new TaskActionSubmission(owner,control,storage);let calls=0;
        await restored.recover(async()=>{calls++;return bad;});
        assert.equal(calls,1);assert.equal(restored.state,'unknown');
    }
});

test('terminal task keeps the original uncertain receipt queryable without re-enabling controls',async()=>{
    const storage=store();let original:any;
    const first=new TaskActionSubmission(owner,taskControl(part,owner)!,storage);
    await first.submit('guidance','Before completion',async(_p,b)=>{original=b;throw Error('lost');});
    const terminal=taskControl({...part,value:{...part.value,state:'sealed'}},owner)!;
    assert.equal(terminal.guidance,false);assert.equal(terminal.cancellation,false);
    const restored=new TaskActionSubmission(owner,terminal,storage);
    await restored.recover(async(path,body)=>{assert.ok(path.endsWith('/receipt'));assert.deepEqual(body,original);return receipt(body,'delivered');});
    assert.equal(restored.state,'delivered');
});


test('lost cancellation receipt settles only from the exact owned Core publication', async () => {
    const storage = store(), control = taskControl(part, owner)!;
    const initial = new TaskActionSubmission(owner, control, storage);
    await initial.submit('cancel', '', async () => { throw new Error('connection lost'); });
    const restored = new TaskActionSubmission(owner, control, storage);
    assert.equal(restored.state, 'unknown');
    restored.refreshControl({...control, cancellation: false});
    assert.equal(restored.state, 'unknown', 'capability removal is not a cancellation receipt');
    restored.refreshControl({...control, identity: 'foreign', cancellationObserved: true});
    assert.equal(restored.state, 'unknown');
    const stopping = taskControl({...part, value: {...part.value, cancel_requested: true}}, owner)!;
    restored.refreshControl(stopping);
    assert.equal(restored.state, 'unknown', 'persisted intent does not clear an unconfirmed stop');
    const terminal = taskControl({...part, value: {...part.value, state: 'cancelled', cancel_requested: true}}, owner)!;
    restored.refreshControl(terminal);
    assert.equal(restored.state, 'ready', 'terminal task owns its final status, no stale pending banner');
    assert.equal(new TaskActionSubmission(owner, terminal, storage).state, 'ready');
});

test('terminal Core observation wins over an in-flight cancellation transport failure', async () => {
    const control = taskControl(part, owner)!, submission = new TaskActionSubmission(owner, control, store());
    let fail!: (error: Error) => void;
    const pending = submission.submit('cancel', '', () => new Promise((_, reject) => { fail = reject; }));
    submission.refreshControl(taskControl({...part, value: {...part.value, state: 'completed'}}, owner)!);
    fail(new Error('lost response'));
    await pending;
    assert.equal(submission.state, 'ready');
});


test('two tabs serialize intents and late guidance cannot remove a newer cancellation', async () => {
    const storage = store(), control = taskControl(part, owner)!;
    let tail: Promise<unknown> = Promise.resolve();
    const lock: TaskIntentLock = (_name, operation) => {
        const result = tail.then(operation); tail = result.catch(() => {}); return result;
    };
    const first = new TaskActionSubmission(owner, control, storage, lock);
    const second = new TaskActionSubmission(owner, control, storage, lock);
    let release!: () => void;
    const barrier = new Promise<void>(resolve => { release = resolve; });
    let count = 0;
    const pending = first.submit('guidance', 'Original', async (_path, body) => {
        count++; await barrier; throw new Error('response lost');
    });
    const competing = second.submit('guidance', 'Replacement', async () => { count++; return {}; });
    release(); await Promise.all([pending, competing]);
    assert.equal(count, 1);
    assert.equal(second.pendingGuidanceText, 'Original');
    await second.recover(async (_path, body) => receipt(body));
    assert.equal(second.state, 'queued');
});

test('cancel recovery checks exact historical owner before monotonic retry', async () => {
    const storage = store(), control = taskControl(part, owner)!;
    const first = new TaskActionSubmission(owner, control, storage);
    await first.submit('cancel', '', async () => { throw Error('offline'); });
    const restored = new TaskActionSubmission(owner, control, storage);
    const calls: string[] = [];
    await restored.recover(async (path, body) => {
        calls.push(path);
        assert.deepEqual(body, {hostId:'host', conversationId:'conversation', parentRunId:'run', taskId:'task'});
        return {...body, cancelRequested:!path.endsWith('/receipt'), terminal:false, state:'running', applicationPending:false};
    });
    assert.deepEqual(calls, ['/api/chat/agent/v2/task/cancel/receipt','/api/chat/agent/v2/task/cancel']);
    assert.equal(restored.state, 'cancelRequested');
});


test('stop failure remains recoverable despite persisted cancellation intent', async () => {
    const submission = new TaskActionSubmission(owner, taskControl(part, owner)!, store());
    const calls: string[] = [];
    await submission.submit('cancel', '', async (path, body) => {
        calls.push(path);
        if (!path.endsWith('/receipt')) throw Error('External target requires independent cancellation reconciliation');
        return {...body, cancelRequested:true, terminal:false, state:'running', applicationPending:false};
    });
    assert.equal(submission.state, 'unknown');
    assert.equal(submission.canRecover, true);
    assert.deepEqual(calls, ['/api/chat/agent/v2/task/cancel/receipt','/api/chat/agent/v2/task/cancel']);
});
